# Coordinate the application with Tokio

Vertui will use Rust async/await on a Tokio runtime to coordinate terminal input, refresh timers, provider command results, and rendering without blocking interaction. Following Herdr's smaller applicable pattern, blocking terminal input may run on a dedicated thread and publish events through a channel, while the main Tokio loop owns state updates; Vertui will not inherit Herdr's unrelated PTY, server, or client complexity.
