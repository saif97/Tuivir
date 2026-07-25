# Align equivalent keybindings with lazydocker

Virtui will preserve lazydocker's keybindings when an equivalent operation exists, while allowing provider-specific commands within each provider workspace. Commands remain keyboard-first and discoverable through a contextual `?` help overlay generated from the registered keybindings; destructive operations still require confirmation. This reduces relearning for lazydocker users without forcing unrelated Docker, Incus, and future provider operations into identical command sets.
