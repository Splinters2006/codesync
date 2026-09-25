# Access from outside home

In **Edit server / passwords**, enter the local / primary address and its SSH port.
You can also enter a public address and a separate public SSH port. IPv4, IPv6,
hostnames, and SSH aliases are supported. Put ports in the port fields, not in the
address. IPv6 may be entered with or without brackets.

## Choosing the connection

- **Automatic:** try Tailscale, then local / primary, then public.
- **Local only:** use only the local / primary address.
- **Remote only:** try Tailscale, then public; never try the local address.

Every connection must match the same saved SSH server key. Codesync verifies the
identity and authentication before transferring files. An unrelated server using
the same private IP on another network is rejected, and Automatic can try the next
configured address. A reachable IP alone is never sufficient.

Before the first sync, use **Test connection** and confirm the fingerprint against
your intended server. You can display its ED25519 fingerprint on the server with:

```sh
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

Use the matching host-key type if SSH presents another type. During this first
confirmation there is no automatic fallback: select Local only or Remote only to
choose which address to enroll. Subsequent connections use strict key checking.
A mismatch never triggers a new trust prompt or automatic key replacement.

GUI identities are stored by server ID in `~/.ssh/codesync_known_hosts`, so the
same identity applies across all addresses and ports. Existing installations need
to confirm their GUI server identity once after this update. The CLI retains its
normal SSH known-hosts behavior. Removing and re-adding a server creates a new
identity; ordinary address edits preserve the existing identity.

## Set up Tailscale from Codesync

1. Start while the server is reachable through an existing address, such as its
   home LAN address. Enter the server's login and sudo credentials.
2. Click **Open** beside the server, then **Set up Tailscale** in its settings
   popup. The button saves the form first.
3. Enter the host's sudo password in the dedicated dialog if requested. Codesync installs Tailscale
   if missing and enables its service on the host and server. The remote step
   uses the saved sudo password, root, or passwordless doas.
4. When **Sign in to Tailscale - This host** or **Sign in to Tailscale - Server**
   appears above Activity, click it and complete browser sign-in. Use the same
   Tailscale network for both devices. Each sign-in attempt waits up to two minutes;
   retry setup if it times out.
5. Codesync reads the server's Tailscale address and actual SSH listening port,
   then verifies SSH over that address against its saved identity. Only after that
   succeeds is the Tailscale address saved. Select Automatic or Remote only to use it.

The host needs `sudo` (or, as a fallback, `pkexec` with a running desktop Polkit
authentication agent). The host password is separate from the server password
and is not saved. An already-connected device skips installation and authorization.
Installation supports Arch through pacman and Alpine through apk. Other supported
Linux distributions use [Tailscale's official installer](https://tailscale.com/docs/install/linux)
(downloaded over HTTPS using curl or wget). systemd and OpenRC service management
are supported. Both machines need internet access and permission to install packages.

Setup preserves the primary and public addresses as fallbacks. If installation or
sign-in fails, completed installation steps remain; setup can be retried. Stop does
not uninstall packages or disconnect an already-connected Tailscale device.
Codesync does not enable Tailscale SSH or alter existing Tailscale preferences with
`--reset`; it uses your existing SSH service. If an existing Tailscale configuration
requires additional `tailscale up` flags, resolve that in a terminal and retry.

The browser completes sign-in through Tailscale; Codesync never stores your
Tailscale account password. Login URLs are shown as labeled buttons, not printed
in Activity. Address discovery uses captured output, so there is no new IP preview.
See the [official sign-in command documentation](https://tailscale.com/docs/reference/tailscale-cli/up).

## Direct public address

For a home server behind an IPv4 router, forward an external TCP port to the
server's SSH port and allow it through the firewall. Give the server a stable LAN
address. Enter the router's public IP or dynamic DNS hostname in **Public address**
and the external port in **Public SSH port**. For example, a forward from TCP 2222
to the server's TCP 22 means the public port is 2222 and the local port is 22.

Carrier-grade NAT can prevent ordinary port forwarding from working; use Tailscale
or ask your ISP about a public address. Public IPv6 requires IPv6 connectivity on
both ends and a firewall rule allowing SSH. For a VPS, allow SSH in the server and
cloud firewalls. SSH keys are recommended for internet-facing SSH.

Codesync does not change router settings. Test from another network, such as a
phone hotspot, before relying on access from class. If your public IP changes,
update it or use a dynamic DNS hostname.
