use std::{
    collections::BTreeSet,
    io::Read,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_HOSTS: u32 = 4096;
#[derive(Clone)]
pub struct Host {
    pub address: Ipv4Addr,
    pub port: u16,
}
pub enum Event {
    Found(Host),
    Progress(usize, usize),
    Finished(bool),
}
pub struct Scan {
    pub events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Scan {
    pub fn start(cidr: &str, port: u16, ctx: eframe::egui::Context) -> Result<Self, String> {
        if port == 0 {
            return Err("Choose an SSH port from 1 to 65535.".into());
        }
        let targets = targets(cidr)?;
        let (send, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        thread::spawn(move || {
            let next = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            let _ = send.send(Event::Progress(0, targets.len()));
            thread::scope(|scope| {
                for _ in 0..32.min(targets.len()) {
                    let next = &next;
                    let done = &done;
                    let send = &send;
                    let signal = &signal;
                    let targets = &targets;
                    let ctx = &ctx;
                    scope.spawn(move || {
                        while !signal.load(Ordering::Relaxed) {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(address) = targets.get(index) else {
                                break;
                            };
                            if ssh_banner(SocketAddr::from((*address, port))) {
                                let _ = send.send(Event::Found(Host {
                                    address: *address,
                                    port,
                                }));
                            }
                            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                            let _ = send.send(Event::Progress(finished, targets.len()));
                            ctx.request_repaint();
                        }
                    });
                }
            });
            let _ = send.send(Event::Finished(signal.load(Ordering::Relaxed)));
            ctx.request_repaint();
        });
        Ok(Self { events, cancel })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn targets(cidr: &str) -> Result<Vec<Ipv4Addr>, String> {
    let (address, prefix) = cidr
        .trim()
        .split_once('/')
        .ok_or("Enter an IPv4 network, for example 192.168.1.0/24.")?;
    let address: Ipv4Addr = address.parse().map_err(|_| "Enter a valid IPv4 network.")?;
    let prefix: u32 = prefix.parse().map_err(|_| "Invalid network prefix.")?;
    if !(20..=32).contains(&prefix) {
        return Err(
            "Scan at most 4096 addresses at a time: choose a /20 or smaller IPv4 network.".into(),
        );
    }
    let count = 1u32 << (32 - prefix);
    if count > MAX_HOSTS {
        return Err("Network is too large.".into());
    }
    let base = u32::from(address) & (u32::MAX << (32 - prefix));
    let range = if prefix <= 30 { 1..count - 1 } else { 0..count };
    let addresses: Vec<_> = range
        .map(|offset| Ipv4Addr::from(base + offset))
        .filter(|ip| !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast())
        .collect();
    if addresses.is_empty() {
        return Err("This network has no scannable IPv4 addresses.".into());
    }
    Ok(addresses)
}

// Read the public SSH protocol greeting only. Never authenticate or send credentials.
fn ssh_banner(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(600)) else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut data = Vec::new();
    let mut buffer = [0u8; 512];
    while data.len() < 4096 {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        if stream.set_read_timeout(Some(remaining)).is_err() {
            return false;
        }
        let Ok(count) = stream.read(&mut buffer) else {
            return false;
        };
        if count == 0 {
            return false;
        }
        data.extend_from_slice(&buffer[..count]);
        for line in data.split_inclusive(|byte| *byte == b'\n') {
            if line.ends_with(b"\n")
                && (line.starts_with(b"SSH-2.0-") || line.starts_with(b"SSH-1.99-"))
            {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
pub fn local_networks() -> Vec<String> {
    let mut networks = BTreeSet::new();
    // getifaddrs owns the linked list until freeifaddrs; only AF_INET pointers are cast.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return Vec::new();
        }
        let mut current = list;
        while !current.is_null() {
            let interface = &*current;
            if !interface.ifa_addr.is_null()
                && !interface.ifa_netmask.is_null()
                && (*interface.ifa_addr).sa_family as i32 == libc::AF_INET
                && interface.ifa_flags & libc::IFF_UP as u32 != 0
                && interface.ifa_flags & (libc::IFF_LOOPBACK | libc::IFF_POINTOPOINT) as u32 == 0
            {
                let address = &*(interface.ifa_addr as *const libc::sockaddr_in);
                let mask = &*(interface.ifa_netmask as *const libc::sockaddr_in);
                let mask = u32::from_be(mask.sin_addr.s_addr);
                let prefix = mask.leading_ones();
                let base = Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr) & mask);
                networks.insert(format!("{base}/{prefix}"));
            }
            current = interface.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    networks.into_iter().collect()
}

#[cfg(windows)]
pub fn local_networks() -> Vec<String> {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            "Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.AddressState -eq 'Preferred' -and $_.IPAddress -ne '127.0.0.1' } | ForEach-Object { '{0}/{1}' -f $_.IPAddress,$_.PrefixLength }"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let mut networks = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((address, prefix)) = line.trim().split_once('/') else {
            continue;
        };
        let (Ok(address), Ok(prefix)) = (address.parse::<Ipv4Addr>(), prefix.parse::<u32>()) else {
            continue;
        };
        if prefix > 32 {
            continue;
        }
        let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
        networks.insert(format!(
            "{}/{}",
            Ipv4Addr::from(u32::from(address) & mask),
            prefix
        ));
    }
    networks.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subnet_bounds_and_validation() {
        let hosts = targets("192.168.1.42/24").unwrap();
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(hosts[253], Ipv4Addr::new(192, 168, 1, 254));
        assert_eq!(targets("192.168.1.0/31").unwrap().len(), 2);
        assert_eq!(targets("192.168.1.1/32").unwrap().len(), 1);
        for invalid in [
            "10.0.0.0/8",
            "1.2.3.4/33",
            "::1/128",
            "bad",
            "0.0.0.0/32",
            "224.0.0.1/32",
        ] {
            assert!(targets(invalid).is_err());
        }
    }
    #[test]
    fn identifies_ssh_without_sending_data_or_accepting_other_services() {
        use std::io::Write;
        for (banner, expected) in [
            ("SSH-2.0-Test\r\n", true),
            ("Welcome\r\nSSH-1.99-Test\r\n", true),
            ("HTTP/1.1 200 OK\r\n", false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (mut connection, _) = listener.accept().unwrap();
                connection.write_all(banner.as_bytes()).unwrap();
                connection.shutdown(std::net::Shutdown::Write).unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                assert_eq!(connection.read(&mut [0u8; 1]).unwrap(), 0);
            });
            assert_eq!(ssh_banner(address), expected);
            server.join().unwrap();
        }
    }
}
