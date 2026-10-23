# SFTP, port forwarding and snippets

English | [Русский](ru/05-sftp-forwarding-snippets.md)

## SFTP

The **SFTP** tab (Ctrl+2) is a file manager with two panels. The left one
starts on your computer's files; open a host in either panel and copy files
between them.

![The SFTP tab](images/sftp-local.png)

### Opening a host

1. In an empty panel, click **Choose host…** (or, in a panel that shows
   files, its source button > **Host…**).
2. Pick a saved host; the search filters by label or address.
3. The panel connects. Password, passphrase and host key questions appear
   in the panel itself.

The source button switches a panel back to **Local files**; **Close** in
the panel's **…** menu disconnects it.

### Moving around

- Double-click a folder to open it; double-click the **..** row (or press
  Backspace in the list) to go up. The arrows above the list go back and
  forward through the folders you opened, and every part of the path is
  clickable; on Windows the first part lists the drives.
- **Filter** narrows the list by name. Click a column header to sort.
- Ctrl+click and Shift+click select several items; Ctrl+A selects all.
- **Show Hidden Files** in the **…** menu, or for every panel in Settings >
  **SFTP**.

### Copying files

Files are always copied into the folder the **other** panel shows:

- drag them from one panel onto the other;
- right-click > **Copy to target directory**;
- or drop files from Windows Explorer onto a panel.

Folders are copied with everything inside. Between two hosts, only files
can be copied in this version.

![The transfer queue](images/sftp-queue.png)

Transfers appear in the queue under the panels with their speed and time
left. Each can be paused, resumed, retried or cancelled; **Clear finished**
tidies the list. A single file between this computer and a server continues
where it stopped; a folder, or a copy between two hosts, starts over. Three run at a time by
default (Settings > **SFTP** > **Transfers at the same time**).

When a file with the same name exists, Shell++ asks: **Skip**, **Keep both**
(the copy becomes `name (2).ext`) or **Replace**; tick **Apply to all** for
the rest of the transfer. Settings > **SFTP** can make it always replace,
skip or rename instead. Only the item you copy is checked: copying a folder
onto a folder of the same name with **Replace** merges them, and files
inside that have the same names are replaced without asking. While
Shell++ is locked, transfers go on, and such a question waits until you
unlock.

### Other file actions

The right-click menu has **Open**, **Open with…**, **Rename…**,
**Delete…**, **New Folder…** and **Edit Permissions…** (read, write and
execute for owner, group and others, or an octal value, also for
everything inside a folder; not for local files on Windows).

![File actions](images/sftp-actions.png)

**Open** on a server file downloads a copy into
`%LOCALAPPDATA%\shellpp\tmp\` and opens it with its program (or the one set
in Settings > **SFTP** > **Program for Open**). When you save the file
there, Shell++ uploads it back; if the file changed on the server in the
meantime, it asks which one to keep. The copy is not encrypted. It is
deleted when its SFTP session ends (if nothing is waiting to be uploaded),
and the whole `tmp` folder is emptied when Shell++ starts or exits, without
asking: save and close the file in its program before you quit Shell++.
**Open** on a program file (`.exe`, `.ps1`, `.sh` and so on) asks first,
because it would run on your computer; so does a link on the server that
points to one, whatever its own name.

> File names on a server that are not valid UTF-8 are shown with `�` and
> cannot be opened, renamed or downloaded.

## Port forwarding

The **Port Forwarding** screen (in the sidebar) keeps forwarding rules that
carry connections through an SSH host:

- **Local**: a port on this computer reaches a service behind the SSH
  host, for example a database that only listens on the server
  (`127.0.0.1:5432` there, through `localhost:8080` here);
- **Remote**: a port on the SSH host reaches a service on this computer
  or your network;
- **Dynamic** (SOCKS): a SOCKS proxy on this computer sends any
  connection through the SSH host.

![Forwarding rules](images/forwarding-cards.png)

### Creating a rule

**New forwarding** starts a wizard:

1. the type, with a picture of where the connection goes;
2. the SSH host it goes through;
3. where the port opens (address and port): on this computer, or for a
   Remote rule on the SSH host. `127.0.0.1` keeps the port to that machine;
   another address shows a warning;
4. where its connections go (not for SOCKS);
5. a label, and a one-line summary such as
   `127.0.0.1:8080 → 127.0.0.1:5432 via web-1`. **Create** saves the rule;
   **Create and start** also starts it.

![The forwarding wizard](images/forwarding-wizard-type.png)

**Skip wizard**, or the arrow next to **New forwarding**, opens the full
form instead. The full form also has **Start automatically**: **When I
connect to the host** or **When the app starts**.

### Using rules

- Double-click a card, or select it and press Enter, to start or stop the
  rule. The card shows **Running**, **Stopped**, a reconnect attempt, or
  why it failed (for example "Port 8080 is busy. Choose another port.").
- Click a card to edit the rule; saving a running rule restarts it.
- The right-click menu has **Start**/**Stop**, **Edit**, **Duplicate** and
  **Remove** (with **Undo**).

> A rule connects without a tab, so it cannot ask you anything. Before you
> start one, connect to its host (and its jump hosts) once in a tab and
> choose **Accept and save** for the host key (**Connect once** is not
> enough). The rule then signs in with the host's saved password or key,
> the SSH agent, or your `~/.ssh` key files (one with a passphrase only if
> you let Shell++ remember it in this run). Prompts such as one-time codes
> cannot be answered.

## Snippets

Snippets are commands and scripts you keep at hand: on the **Snippets**
screen (in the sidebar), in a session's terminal sidebar, and on several
hosts at once.

![Snippets](images/snippets.png)

### Creating a snippet

1. Click **New snippet**.
2. Enter an **Action description** (for example "Restart nginx") and the
   **Script**. Optionally put it in a **Package** (a folder of snippets) and
   add tags.
3. Use `{{name}}` for a value that changes: Shell++ asks for it each time
   the snippet runs, and does not store it. The same name is asked once.
   The value is inserted exactly as typed, so quote it in the script if it
   can contain spaces.
4. Optionally add **Targets for execution**: the hosts the snippet runs on.
5. Click **Create**.

![A snippet](images/snippet-panel.png)

### Generating a script from a description (beta)

Shell++ can write a snippet's script from a description, with an
OpenAI-compatible service you choose: a hosted one (OpenAI and many
others) or a local model (Ollama, llama.cpp, LM Studio). It is off by
default.

1. Settings > **Terminal** > **Script generation (beta)**: turn it on, enter
   the **Address** of the API (empty means `https://api.openai.com/v1`; a
   local model is for example `http://localhost:11434/v1`) and the
   **Model**. For a hosted service, enter the **API key** and click **Save
   key**: it is kept in the system's credential store for that address's
   host and sent only there; a local model needs none.
2. In the snippet form, the **Generate from a description (beta)** row
   appears. Describe what the script should do and click **Generate**. The
   line under it names the host the description goes to.
3. The script is added to the **Script** field. It is never run or saved
   by itself: read it, then **Create** or **Save**.

The description is sent only when you press **Generate**, and only the
description (no hosts, passwords or keys). An Import Data file never
changes these settings.

### Using a snippet in a session

Open the **Terminal sidebar** in a session tab and its **Snippets** page.
Click a snippet to type it at the prompt without running it, or use its run
button to type it and press Enter. A script of several lines in a shell
that would run each line at once can only be run, not inserted.

### Running a snippet on several hosts

**Run** (in the snippet's panel, its card menu, or a double-click on its
card) runs the script on its targets and shows the results: for each host,
success, error or timed out, and the exit code; click a host to see what
the script printed. With two or
more hosts Shell++ lists them and asks first; for two to four hosts, **Run
in split tab** instead opens one pane per host and types the script in
each.

![Run results](images/snippet-results.png)

Runs on several hosts connect without a tab, like port forwarding rules,
with the same conditions (host keys saved with **Accept and save**, saved
credentials, the agent or key files). Up to 8 hosts run at a time, each for
at most a minute.

### Shell history

**Shell History** on the Snippets screen lists the commands you typed in
sessions to saved hosts, newest first. The **Save as snippet** button of a
command opens a new snippet with it, for that host; click **Create** to
keep it. Commands that look like secrets (tokens, passwords
in a command line) are never kept. Turn the history off in Settings >
**Terminal** > **Record command history**, or per host in the host panel.

## Logs

The **Logs** screen lists your SSH and SFTP sessions to hosts (SFTP ones
show as SSH): when, to which host and as whom, and how each one ended.
Port forwarding rules and snippet runs on several hosts are not listed. Click an entry for its details.
Settings > **Logs** sets how long entries are kept (90 days by default;
entries you mark as saved stay).

**Record session output** (Settings > **Logs**, off by default) records the
output of SSH sessions, encrypted and on this device only. A recording
opens from its log entry (**Open recording**), with search and an export to
a text file.
