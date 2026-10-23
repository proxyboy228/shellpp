# Keys, the Keychain and Export to host

English | [Русский](ru/03-keys.md)

The **Keychain** (sidebar item "Keychain") holds your SSH private keys and
identities. Everything in it is stored encrypted in your vault.

![The Keychain](images/keychain.png)

## Signing in: which keys are tried

When you connect to a host, Shell++ tries these in order:

1. the key set on the host;
2. the password saved on the host;
3. the keys of the system SSH agent: `SSH_AUTH_SOCK`, the Windows OpenSSH
   agent or Pageant, found automatically;
4. the key files `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa` and other
   `~/.ssh/id_*` files. A passphrase is asked only after the server
   accepts the key, and you can tick **Remember until Shell++ locks or
   exits**;
5. a password prompt;
6. keyboard-interactive prompts (for example a one-time code).

At most four public keys are offered to a server in all. If a server
refuses you because of that, set the right key on the host.

## Generating a key

1. In the Keychain, open the arrow next to **New key** and choose
   **Generate key**. On an empty Keychain, use **Generate**.
2. Choose the **Key type**: Ed25519 (the default and the best choice),
   ECDSA (curve P-256, P-384 or P-521) or RSA (3072 or 4096 bits).
3. Enter a **Label**, for example "work laptop". It is required.
4. Optionally enter a **Passphrase** and repeat it. With a passphrase you
   choose where it lives:
   - **Save in the vault**: the key opens by itself when the vault is
     unlocked;
   - **Ask on every connection**: Shell++ asks for it each time.
5. Click **Generate**. An RSA key takes a few seconds. The new key's panel
   opens.

![Generating a key](images/keychain-generate.png)

## Importing a key you already have

Click **New key**. The import form takes a key in any of these ways:

- paste the key text into the field;
- **Choose file…**;
- drop the file onto the form.

Shell++ reads the key as you paste it and shows its **Format**:

- OpenSSH (plain or encrypted);
- PEM: PKCS#1, PKCS#8 (also encrypted) and SEC1, including legacy OpenSSL
  encryption with AES-CBC;
- PuTTY `.ppk`, versions 2 and 3, encrypted or not.

Supported key types are Ed25519, ECDSA P-256/P-384/P-521 and RSA of 2048
bits or more. A key protected by a passphrase asks for it ("Protected by a
passphrase: enter it to import the key") and offers the same **Save in the
vault** / **Ask on every connection** choice. PEM and PuTTY keys are
converted to the OpenSSH format and keep their passphrase.

The **Label** is optional: without one, the key's comment is used.

What is refused, and why:

- DSA, Ed448 and other types: "The key type … is not supported."
- RSA keys under 2048 bits.
- Keys encrypted with DES-EDE3.
- Public keys (`.pub`), certificates and RFC 4716 files: "This is a public
  key. Import the private key: the file of the same name without .pub."
- Files over 64 KB.

A key that is already in the Keychain (the same SHA256 fingerprint) is not
added twice: its panel opens with "This key is already in the Keychain".

### Importing from `~/.ssh`

On an empty Keychain, **Import from SSH folder** lists the private keys in
`~/.ssh` (files in a supported format, not `*.pub`). Click one to load it
into the import form. Once the Keychain holds a key, add more keys from
`~/.ssh` with **New key** > **Choose file…**, or import your OpenSSH
config: its `IdentityFile` keys are imported with the hosts
([Hosts](02-hosts.md#importing-from-an-openssh-config)).

## The key panel

Click a key to open its panel ("Key details"):

![The key panel](images/keychain-key-panel.png)

- **Label**: saved as you type.
- **Private key**: its type and SHA256 fingerprint (with a copy button).
  The key text stays hidden until you click **Show**. **Copy private key**
  puts it on the clipboard and clears the clipboard after 30 seconds. The
  key is hidden again when you leave the panel or the app locks. A line
  under it says whether it has a passphrase and where the passphrase is
  kept.
- **Public key**: one line, with a copy button. Paste it into a server's
  `~/.ssh/authorized_keys`, or use Export to host (below).
- **Certificate**: shown when a certificate is attached.
- **Key export**: **Export to host…**.

The **…** menu has **Copy public key** and **Remove**. Removing a key asks
first and lists the hosts, groups and identities that use it; they then
connect with their other sign-in methods. A toast offers **Undo** for a few
seconds. (**Duplicate** and **Export to file…** are not available yet: to
save key files, use Export Data > OpenSSH files, see
[Export and migration](07-export-and-migration.md).)

## Export to host

Export to host adds a public key to `~/.ssh/authorized_keys` on servers you
choose, like `ssh-copy-id`.

1. Open the key's panel and click **Export to host…**.
2. Tick the hosts. The search box filters by label or address; **Select
   all** ticks the hosts the search shows.
3. Click **Export**.

Each host then shows its result: **Added**, **Already there**, **Connect
once to save its host key first**, **No saved credentials signed in**, or a
connection error.

How it works:

- No tab opens. Shell++ signs in with what it can use without asking you:
  the host's saved password or key, the system agent and the `~/.ssh/id_*`
  files. A password that is not saved, or a key whose passphrase is asked
  on every connection, cannot be used here: connect once from a tab
  instead.
- Only hosts whose host key is already in Known Hosts are reached, so
  connect to a new host once (and accept its key) before exporting to it.
- On the server, Shell++ creates `~/.ssh` (mode 700) and
  `authorized_keys` (mode 600) if they are missing, and appends the key
  only when it is not there yet (the same key with another comment counts
  as already there).

## Certificates

**Certificate** in the action bar adds an OpenSSH user certificate (the
`*-cert.pub` file): paste it or choose the file. It is attached to the key
it certifies, whose private key must be in the Keychain already. The key
panel then shows the certificate's key ID, principals, validity and CA
fingerprint, and warns when it has expired.

> Certificates are stored, shown and exported, but this version does not
> sign in with them yet.

## Identities

An identity is a user name with a password, which several hosts can share.
Create one from a host's panel: **+** (Add identity, key or certificate) >
**Identity** > **New identity**, with a name, the user name and the
password. Identities appear in the Keychain under **Identities**; this
version shows them there but does not edit them yet.

## Agent forwarding

Agent forwarding lets a server use your keys for the next hop (for example
`git` or `ssh` to another machine) without copying them there. It is off by
default and set per host: host panel > **Show more** > **Agent
Forwarding**.

![Agent forwarding in the host panel](images/agent-forwarding.png)

- While it is on, anyone with root on that host can sign in elsewhere with
  your keys while your session is open. Turn it on only for hosts you
  trust.
- Forwarded are the Keychain keys that open without a prompt (no
  passphrase, or a saved one). **Also offer the system agent's keys** adds
  the agent's keys.
- **Ask before every signature**: each signature the server asks for
  waits for your **Allow** in the session's tab ("Sign with your key?").
  No answer within two minutes counts as **Deny**. When the window is in
  the background, a system notification names the host.
- The server can only list keys and ask for signatures: it cannot add,
  remove or read them. While Shell++ is locked, the Keychain keys are not
  offered (the system agent's keys still are, if you turned them on).
