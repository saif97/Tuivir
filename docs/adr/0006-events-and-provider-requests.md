# Separate application events from provider requests

Tuivir will model facts received by the application as specific `AppEvent` variants and asynchronous provider work as specific `ProviderRequest` variants. Key presses, refresh timers, and completed provider work are events; refreshing a Provider Workspace and executing a Resource Command are provider requests. The main loop handles events, updates state, and dispatches requests, while process execution returns completion events.
