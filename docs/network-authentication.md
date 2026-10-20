# Network authentication

Aizome's network controls and NetworkManager's credential agent are
separate clients of NetworkManager. NetworkManager manages the devices,
connections, supplicant and VPN services; it does not create a tray icon.
`nm-applet` provides both another network UI and a credential agent.

`aizome-network-agent.service` now supplies the credential agent for an
Aizome session. It starts with `aizome-session.target`, independently of
the shell, and ends with that session. Reloading the shell leaves requests
and dialogs intact. After NetworkManager restarts, the agent registers
with its new D-Bus owner.

The NixOS module prevents `nm-applet` from starting in Aizome through
either its systemd user service or the XDG autostart entry. An override
in `/etc/xdg/aizome/autostart` disables that startup entry; it does not
filter tray items. The shared desktop entry and installed applet remain
available to other desktops. The shell displays status items normally.
An explicitly launched `nm-applet` can still show an icon.

## Credentials

The agent implements NetworkManager's `GetSecrets`, `CancelGetSecrets`,
`SaveSecrets` and `DeleteSecrets` methods. Each call must come from the
current owner of `org.freedesktop.NetworkManager` on the system bus.
The agent opens native dialogs and permits its keyring reads to unlock a
collection only when NetworkManager allows interaction. VPN helpers
receive the same interaction flag. Requests for replacement credentials
do not reuse cached Wi-Fi passwords.

Native dialogs cover Wi-Fi passwords and WEP keys, enterprise passwords,
private-key passwords and PINs, mobile/PPP passwords, and WireGuard private
and peer preshared keys. Enterprise connection configuration (EAP method,
identity, CA and server validation) stays in the existing profile.

Plugin VPNs use the installed plugin's declared authentication helper.
Its hints, challenge fields and retry flags are passed through. Plugins
supporting the external UI protocol get Aizome dialogs; others keep their
plugin-provided UI. A missing plugin fails the request rather than
guessing which credentials it needs. The NetworkManager VPN plugin must
be installed, including its authentication helper.

Existing profile policies are preserved:

| Secret policy | Behavior |
| --- | --- |
| System-owned (`0`) | Return credentials to NetworkManager; do not copy them into the login keyring. |
| Agent-owned (`1`) | Read and save through Secret Service/libsecret. |
| Not saved (`2`, including `1\|2`) | Ask as needed; do not persist the supplied secret. |
| Not required (`4`) | Do not request that credential. |

The keyring uses NetworkManager's established `connection-uuid`,
`setting-name` and `setting-key` attributes, so credentials saved by other
desktops remain usable. Saving an edited profile replaces its saved
credentials; deleting a profile removes them. A keyring save failure is
reported in the service journal without preventing use of the supplied
password. A locked keyring may request its own unlock password during an
interactive request.

The existing join controls can still supply system-owned passwords
directly. The agent handles later requests: reconnects, changed passwords,
ask-every-time profiles, enterprise credentials and VPN authentication.

Credentials reach worker processes through pipes, not command arguments,
environment variables or temporary files. Cancellation terminates pending
dialogs/helpers and their process groups; workers have bounded input,
output and a two-minute deadline. Core dumps are disabled for the service.
Password fields omit their contents from UI inspection and accessibility.

## Verification

Run the focused tests with:

```sh
nix develop -c cargo test -p aizome-network-agent
nix develop -c cargo test -p aizome-kit --test text_field
nix develop -c cargo test -p aizome-settei --test network
nix build .#checks.x86_64-linux.network-auth -L
```

The VM uses `mac80211_hwsim` radios, WPA2 and WPA3 access points, a PEAP
enterprise server, a real OpenVPN server/plugin, and a private login
keyring unlocked through a normal greetd/PAM password login. Tests
operate the native dialogs through the compositor and
verify activation, storage policies, compatibility with existing keyring
entries, wrong passwords, cancellation, deletion, shell reloads,
NetworkManager restarts and both applet startup paths. Fixture credentials
and servers exist only in the test VM. These tests do not manipulate the
host's interfaces, desktop session or NetworkManager service.

Protocol tests additionally cover unauthorized callers, VPN challenges,
legacy helpers, noninteractive requests, storage flags and cancellation
of blocked helper processes. Testing one VPN plugin does not establish
compatibility with every third-party plugin or authentication provider.
