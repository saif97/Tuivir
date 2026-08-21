use tuivir::{
    application::{InteractiveShellProcess, ResourceShellSessionId},
    infrastructure::resource_shell::{ResourceShellRuntime, ResourceShellRuntimeEvent},
};

#[test]
fn a_real_pty_shell_wakes_the_host_and_keeps_its_rendered_output() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = ResourceShellSessionId::new(1);

    runtime
        .start(
            session,
            &InteractiveShellProcess::new("/bin/sh", &["-c", "printf 'hello from pty\\n'"]),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");

    let woke_for_output = (0..3).any(|_| {
        matches!(
            receiver.blocking_recv().expect("PTY output wakes the host"),
            ResourceShellRuntimeEvent::OutputReady { session_id } if session_id == session
        )
    });
    assert!(woke_for_output, "PTY output must wake the host");
    assert!(
        runtime
            .screen_text(session)
            .expect("live session screen")
            .contains("hello from pty")
    );
}
