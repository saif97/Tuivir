# Embed Resource Shell Sessions in the Details Pane

Supersedes ADR 0009's whole-terminal handover and the handover-specific clauses
of ADR 0010.

Tuivir runs a Provider-declared shell as a Resource Shell Session in a private
PTY and presents it through the Shell Detail View Tab. The session's Provider,
Resource, identity, lifecycle, and presentation belong to application state;
the host owns its child process, PTY, terminal engine, I/O, and cleanup. The
normal Details presentation, enlarged presentation, and hidden state are views
of the same session, never separate shell launch paths.

`E` starts or resumes the selected Resource's session in the enlarged view.
Enter starts the selected Shell Detail View Tab in Details. While terminal input
is focused, ordinary input including `Ctrl-C` belongs to the shell. `Ctrl-B q`
releases input (and restores Details from an enlarged session), while `Ctrl-B z`
changes presentation without restarting the process.

Ordinary Tuivir Quit confirms when any Resource Shell Session remains live. A
confirmed Quit terminates each private PTY process group with a bounded graceful
period, escalates stubborn processes, and reaps the event loop before the host
restores its terminal. This affects only the local Provider CLI processes; it
does not stop or delete their underlying Resources.
