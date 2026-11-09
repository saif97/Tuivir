mod common;

use common::{FixtureCli, resource_target, success};
use tuivir::infrastructure::{
    process::ProcessSpec,
    provider::{DockerWorkspace, ProviderWorkspace},
};

#[tokio::test]
async fn docker_discovers_accounts_only_through_getent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &[
                "exec",
                "web",
                "/bin/sh",
                "-c",
                "if command -v getent >/dev/null 2>&1; then exec getent passwd; else printf 'TUIVIR_GETENT_MISSING\\n'; fi",
            ],
        ),
        success("root:x:0:0:root:/root:/bin/sh\nworker:x:1000:1000::/home/worker:/bin/sh\n"),
    )]);
    let accounts = DockerWorkspace
        .shell_accounts(&cli, &resource_target("containers", "web"))
        .await
        .unwrap();
    assert_eq!(accounts, Some(vec!["root".into(), "worker".into()]));
    cli.assert_exhausted();
}
