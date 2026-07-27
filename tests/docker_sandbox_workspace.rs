use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Mutex,
};

use virtui::{
    cli::{CliRunner, ProcessError, ProcessOutput, ProcessSpec},
    docker_sandbox::DockerSandboxWorkspace,
    provider::ProviderWorkspace,
};

struct FixtureCli {
    responses: Mutex<VecDeque<(ProcessSpec, Result<ProcessOutput, ProcessError>)>>,
}

impl FixtureCli {
    fn new(
        responses: impl IntoIterator<Item = (ProcessSpec, Result<ProcessOutput, ProcessError>)>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl CliRunner for FixtureCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            let (expected, response) = self
                .responses
                .lock()
                .expect("fixture queue lock")
                .pop_front()
                .expect("unexpected CLI command");
            assert_eq!(command, expected);
            response
        })
    }
}

#[tokio::test]
async fn docker_sandbox_is_omitted_when_its_cli_is_absent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["version"]),
        Err(ProcessError::ExecutableNotFound),
    )]);

    assert!(DockerSandboxWorkspace.discover(&cli).await.is_none());
}
