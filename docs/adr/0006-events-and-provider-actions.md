# Separate application events from provider actions

Virtui will model facts received by the application as specific `AppEvent` variants and operations requested from a provider as specific `ProviderAction` variants. Key presses, refresh timers, and completed provider work are events; start, stop, restart, delete, and shell operations are provider actions. The main loop handles events, updates state, and dispatches actions, while process execution returns completion events.
