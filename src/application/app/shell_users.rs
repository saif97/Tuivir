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
        self.open_shell_user_picker_for(key, enlarged)
    }

    pub(super) fn open_shell_user_picker_for(
        &mut self,
        key: ShellUserKey,
        enlarged: bool,
    ) -> Vec<ProviderRequest> {
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
            if choose {
                return self.open_shell_user_picker(enlarged);
            }
            if !live {
                if let Some(saved) = self.state.default_shell_users.iter().find(|s| s.key == key) {
                    return self.verify_shell_user(key, saved.username.clone(), enlarged);
                }
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

impl App {
    pub fn with_default_shell_users(
        mut self,
        users: Vec<crate::application::DefaultShellUser>,
    ) -> Self {
        self.state.default_shell_users = users;
        self
    }

    pub fn take_pending_shell_user_save(
        &mut self,
    ) -> Option<Vec<crate::application::DefaultShellUser>> {
        std::mem::take(&mut self.pending_shell_user_save)
            .then(|| self.state.default_shell_users.clone())
    }

    pub fn report_shell_user_persistence_failure(&mut self, reason: String) {
        self.state.command_error = Some(format!("Saving Default Shell User failed: {reason}"));
    }

    pub(super) fn submit_shell_user(&mut self) -> Vec<ProviderRequest> {
        let Some(picker) = self
            .state
            .shell_user_picker
            .as_mut()
            .filter(|p| !p.loading && !p.verifying)
        else {
            return Vec::new();
        };
        let username = picker
            .candidates()
            .get(picker.selected)
            .copied()
            .unwrap_or(&picker.input)
            .to_owned();
        if username.is_empty() {
            return Vec::new();
        }
        picker.input = username.clone();
        picker.verifying = true;
        picker.error = None;
        let key = picker.key.clone();
        let enlarged = picker.enlarged;
        self.verify_shell_user(key, username, enlarged)
    }

    fn verify_shell_user(
        &mut self,
        key: ShellUserKey,
        username: String,
        enlarged: bool,
    ) -> Vec<ProviderRequest> {
        if self.pending_shell_verification.is_some() {
            return Vec::new();
        }
        let request_id = ProviderRequestId::new(self.next_request_id);
        self.next_request_id += 1;
        self.pending_shell_verification =
            Some(crate::application::shell_user::ShellUserVerification {
                request_id,
                key: key.clone(),
                username: username.clone(),
                enlarged,
            });
        vec![ProviderRequest::VerifyShellUser {
            request_id,
            key,
            username,
        }]
    }

    pub(super) fn apply_shell_user_verified(
        &mut self,
        request_id: ProviderRequestId,
        result: Result<crate::application::ResourceShellProcess, WorkspaceError>,
    ) -> Vec<ProviderRequest> {
        if !self
            .pending_shell_verification
            .as_ref()
            .is_some_and(|p| p.request_id == request_id)
        {
            return Vec::new();
        }
        let pending = self.pending_shell_verification.take().unwrap();
        match result {
            Ok(process) => {
                self.state.shell_user_picker = None;
                let selected = crate::application::VerifiedShellUser {
                    key: pending.key,
                    username: pending.username,
                    process,
                    enlarged: pending.enlarged,
                };
                let live = self
                    .state
                    .live_resource_shell_sessions()
                    .find(|s| {
                        s.provider_id == selected.key.provider_id && s.target == selected.key.target
                    })
                    .cloned();
                match live {
                    Some(session) if session.username.as_deref() == Some(&selected.username) => {
                        self.present_selected_shell(session.id, selected.enlarged);
                    }
                    Some(_) => {
                        self.state.confirmation = Some(Confirmation::ReplaceResourceShell(selected))
                    }
                    None => self.start_verified_shell(selected),
                }
                Vec::new()
            }
            Err(error) => {
                if let Some(picker) = self.state.shell_user_picker.as_mut() {
                    picker.verifying = false;
                    picker.error = Some(error.message);
                    Vec::new()
                } else {
                    let requests = self.open_shell_user_picker_for(pending.key, pending.enlarged);
                    if let Some(picker) = self.state.shell_user_picker.as_mut() {
                        picker.input = pending.username;
                        picker.error = Some(error.message);
                    }
                    requests
                }
            }
        }
    }

    pub(super) fn start_verified_shell(&mut self, selected: crate::application::VerifiedShellUser) {
        use crate::application::DefaultShellUser;
        // A Resource may disappear while a check is running. Never launch a
        // stale target or transfer its choice onto the current selection.
        if !self.state.providers.iter().any(|p| {
            p.id() == &selected.key.provider_id
                && p.resource(&selected.key.target)
                    .and_then(|r| r.shell.as_ref())
                    .is_some()
        }) {
            return;
        }
        if let Some(saved) = self
            .state
            .default_shell_users
            .iter_mut()
            .find(|s| s.key == selected.key)
        {
            saved.username = selected.username.clone();
        } else {
            self.state.default_shell_users.push(DefaultShellUser {
                key: selected.key.clone(),
                username: selected.username.clone(),
            });
        }
        self.pending_shell_user_save = true;
        self.retire_resource_shell_sessions(|s| {
            s.provider_id == selected.key.provider_id && s.target == selected.key.target
        });
        let session = ResourceShellSession {
            id: ResourceShellSessionId(self.next_resource_shell_session_id),
            provider_id: selected.key.provider_id,
            target: selected.key.target,
            username: Some(selected.username),
            lifecycle: ResourceShellSessionLifecycle::Starting,
        };
        self.next_resource_shell_session_id += 1;
        self.present_selected_shell(session.id, selected.enlarged);
        self.state.resource_shell_sessions.push(session.clone());
        self.pending_resource_shell_effects
            .push(ResourceShellEffect::Start {
                session,
                process: selected.process,
            });
    }

    fn present_selected_shell(&mut self, session_id: ResourceShellSessionId, enlarged: bool) {
        if let Some(workspace) = self.state.active_workspace_mut() {
            workspace.select_resource_shell_tab();
        }
        self.state.focused_pane = FocusedPane::Details;
        self.state.resource_shell_presentation = if enlarged {
            ResourceShellPresentation::Enlarged(session_id)
        } else {
            ResourceShellPresentation::Details
        };
    }
}
