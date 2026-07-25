use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::provider::ResourceCommand;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Keybinding {
    pub key: char,
    pub label: String,
}

impl Keybinding {
    fn matches(&self, event: &KeyEvent) -> bool {
        event.code == KeyCode::Char(self.key)
            && matches!(event.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredCommand {
    pub id: &'static str,
    pub description: &'static str,
    pub bindings: Vec<Keybinding>,
    pub command: ResourceCommand,
}

#[derive(Clone, Debug)]
pub struct CommandRegistry {
    resource_commands: Vec<RegisteredCommand>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self {
            resource_commands: vec![
                resource_command("resource.start", "Start", 'S', ResourceCommand::Start),
                resource_command("resource.stop", "Stop", 's', ResourceCommand::Stop),
                resource_command("resource.restart", "Restart", 'r', ResourceCommand::Restart),
                resource_command("resource.resume", "Resume", 'p', ResourceCommand::Resume),
                resource_command("resource.delete", "Delete", 'd', ResourceCommand::Delete),
            ],
        }
    }
}

impl CommandRegistry {
    pub fn resource_command_for_key(&self, event: &KeyEvent) -> Option<ResourceCommand> {
        self.resource_commands
            .iter()
            .find(|command| {
                command
                    .bindings
                    .iter()
                    .any(|binding| binding.matches(event))
            })
            .map(|command| command.command)
    }

    pub fn resource_commands(&self) -> &[RegisteredCommand] {
        &self.resource_commands
    }
}

fn resource_command(
    id: &'static str,
    description: &'static str,
    key: char,
    command: ResourceCommand,
) -> RegisteredCommand {
    RegisteredCommand {
        id,
        description,
        bindings: vec![Keybinding {
            key,
            label: key.to_string(),
        }],
        command,
    }
}
