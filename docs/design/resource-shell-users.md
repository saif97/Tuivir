# Default Shell Users

Implements [#132](https://github.com/saif97/Tuivir/issues/132) within ADRs 0003,
0008, 0010, and 0011.

Shell resumes the live Resource Shell Session. Otherwise, supported Resources
start with their persisted Default Shell User, or ask for an explicit account
choice on first use. Shell as… opens the same picker to change that choice.
Both the enlarged and Details entry points use this path.

The application owns selection, request identities, preferences, confirmation,
and session lifecycle. Provider adapters own discovery, account selection,
verification, and interactive command arguments. The host executes asynchronous
Provider requests, persists XDG state, and owns the PTY. No Provider-specific
branch enters application or presentation code.

Discovery executes only `getent passwd`. A guest-side executable check reports
missing getent explicitly; execution and connectivity failures remain errors.
There is no passwd-file fallback or UID/account-name filtering. Enumeration is
only a source of candidates. The picker also accepts literal usernames.

Submission runs a bounded noninteractive shell command as that one account.
Successful target execution must return the verification acknowledgement;
spawning the local CLI is insufficient. Usernames are passed as arguments.
Failure leaves input, the live session, and the saved preference intact.

After validation, replacing a live session requires confirmation. Selecting its
current account resumes it. Cancelling either modal changes nothing. Confirming
replacement retires the old session and starts one new session, preserving ADR
0011's limit of one Resource Shell Session per Resource.

The host loads and atomically saves Default Shell Users in `shell-users.json`
in Tuivir's XDG state directory. Keys contain Provider identity, optional Target
Environment, Resource Panel identity, and Resource identity. State is independent
of hand-edited configuration and contains no credentials.

Interactive launches acknowledge guest shell execution through a terminal title
sequence consumed by the PTY runtime. A local CLI exit without acknowledgement
is a startup failure, with its terminal diagnostic retained. The host drains
final PTY output before deciding, so a short-lived shell that acknowledges
startup and then exits nonzero remains a normal completed session. Startup
failure reopens selection without silently changing the saved account.

Docker and Apple Container use native `--user`; local Docker Sandbox uses
`sbx exec --user`. Incus uses its existing guest `su -l` path with the selected
username. Tart exposes no native account override and retains native execution.
The Sandbox adapter's `sbx ls --json` selects local sandboxes; cloud enumeration
is outside the current Workspace. Cloud exec rejects `--user`, so future cloud
Resources must keep the account-selection capability disabled.

Account creation, password/sudo setup, alternate discovery, guest-agent changes,
and concurrent sessions for different accounts remain outside this feature.
