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

#[test]
fn resizing_a_live_session_updates_its_pty_before_the_next_command() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = ResourceShellSessionId::new(2);
    runtime
        .start(
            session,
            &InteractiveShellProcess::new("/bin/sh", &["-c", "read ignored; stty size"]),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");

    runtime.resize(session, 42, 12).expect("PTY resizes");
    runtime
        .write(session, b"go\n".to_vec())
        .expect("input reaches shell");
    for _ in 0..4 {
        let event = receiver
            .blocking_recv()
            .expect("resized PTY output wakes the host");
        if matches!(event, ResourceShellRuntimeEvent::OutputReady { session_id } if session_id == session)
            && runtime
                .screen_text(session)
                .expect("live session screen")
                .contains("12 42")
        {
            return;
        }
    }
    let screen = runtime.screen_text(session).expect("live session screen");
    panic!("resized terminal never reported its dimensions:\n{screen}");
}
