use super::*;
use crate::application::{ShellUserKey, ShellUserPicker};

impl App {
    pub(super) fn selected_shell_user_key(&self) -> Option<ShellUserKey> {
        let workspace = self.state.active_workspace()?;
        Some(ShellUserKey {
            provider_id: workspace.id().clone(),
            target_environment: workspace.target_environment().cloned(),
            target: workspace.selected_resource_target()?,
        })
    }

    pub(super) fn open_shell_user_picker(&mut self, enlarged: bool) -> Vec<ProviderRequest> {
        let Some(key) = self.selected_shell_user_key() else {
            return Vec::new();
        };
        let request_id = ProviderRequestId::new(self.next_request_id);
        self.next_request_id += 1;
        self.state.shell_user_picker = Some(ShellUserPicker {
            key: key.clone(),
            input: String::new(),
            accounts: Some(Vec::new()),
            selected: 0,
            loading: true,
            verifying: false,
            error: None,
            request_id,
            enlarged,
        });
        vec![ProviderRequest::DiscoverShellAccounts { request_id, key }]
    }

    pub(super) fn shell_user_selection_supported(&self) -> bool {
        self.selected_resource()
            .and_then(|r| r.shell.as_ref())
            .is_some_and(super::super::ResourceShellProcess::supports_user_selection)
    }

    pub(super) fn request_resource_shell(
        &mut self,
        enlarged: bool,
        choose: bool,
    ) -> Vec<ProviderRequest> {
        if self.shell_user_selection_supported() {
            let key = self.selected_shell_user_key().unwrap();
            let live = self
                .state
                .live_resource_shell_sessions()
                .any(|s| s.provider_id == key.provider_id && s.target == key.target);
            if choose || !live {
                return self.open_shell_user_picker(enlarged);
            }
        } else if choose {
            return Vec::new();
        }
        if enlarged {
            self.open_shell();
        } else {
            self.open_shell_in_details();
        }
        Vec::new()
    }

    pub(super) fn edit_shell_user(&mut self, character: Option<char>) {
        let Some(picker) = self
            .state
            .shell_user_picker
            .as_mut()
            .filter(|p| !p.verifying)
        else {
            return;
        };
        if let Some(c) = character {
            picker.input.push(c);
        } else {
            picker.input.pop();
        }
        picker.selected = 0;
        picker.error = None;
    }

    pub(super) fn move_shell_user(&mut self, delta: isize) {
        let Some(picker) = self
            .state
            .shell_user_picker
            .as_mut()
            .filter(|p| !p.verifying)
        else {
            return;
        };
        let count = picker.candidates().len();
        // The last row always permits the literal input, even if enumeration
        // has partial matches. Directory-service enumeration can be incomplete.
        picker.selected = (picker.selected as isize + delta).clamp(0, count as isize) as usize;
    }
}
