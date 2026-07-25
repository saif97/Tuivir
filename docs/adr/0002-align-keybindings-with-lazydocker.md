# Align equivalent keybindings with lazydocker

Virtui will preserve lazydocker's Keybindings when an equivalent Command exists, while allowing provider-specific Commands within each Provider Workspace. Commands remain keyboard-first and discoverable through a contextual `?` help overlay generated from the registered Keybindings; destructive Commands still require confirmation. This reduces relearning for lazydocker users without forcing unrelated Docker, Incus, and future provider Commands into identical command sets.
