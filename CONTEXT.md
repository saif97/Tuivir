# Virtui

Virtui is a terminal environment for inspecting and operating resources managed by local virtualization and container providers.

## Language

**Provider**:
An installed system that owns and operates resources, such as Docker, Incus, or Docker Sandbox.
_Avoid_: Backend, runtime, engine

**Provider Workspace**:
The provider-specific view of its native resources and operations within Virtui.
_Avoid_: Provider tab, unified resource view

**Active Workspace**:
The single provider workspace currently visible and being refreshed. Inactive workspaces remain idle.
_Avoid_: Selected backend, current tab

**Target Environment**:
The Docker context or Incus remote and project already selected through the provider's CLI configuration.
_Avoid_: Cluster, server, connection

**Command**:
A registered user intention that Virtui can invoke within its Command Scope.
_Avoid_: Action, handler

**Command Scope**:
The structural part of the interface in which a Command may be invoked, such as a focused panel or a Provider Workspace's resource view. Mutable resource state does not change a Command's scope.
_Avoid_: Context, condition

**Keybinding**:
An ordered association between one or more key combinations and a Command. The first combination is the preferred inline hint when the interface has room to show only one.
_Avoid_: Shortcut, hotkey

**Detail View**:
One provider-native way of inspecting a selected Resource, declared by the Provider Workspace that offers it and named in that Provider's own words — Docker's Logs, Stats, and Inspect; Incus's Info, Config, and Console Log. Only the view on screen is ever loaded, and a result that arrives for a Resource or view the user has left is refused rather than shown.
_Avoid_: Tab, pane, inspector, log view

**Resource State**:
What a Provider reported a Resource to be doing at the last refresh, in one vocabulary shared by every Provider: running, stopped, paused, transitioning, broken, or unknown. Each Provider Workspace maps its own status words into it, and an invoked Command carries it so Virtui never asks a Provider CLI for what it already knows. Only _stopped_ is positively determined; every other state, unknown included, means "not settled and stopped", so a Command that must treat those differently fails safe.
_Avoid_: Status, run state, power state, phase

**Interactive Shell**:
A Provider CLI process that temporarily owns Virtui's whole terminal so the user can work inside a Resource directly. Virtui clears its screen and stops reading keys before handing over, and takes both back however that process ends — including one that never started. The shell runs on Virtui's own screen rather than on the terminal Virtui was launched from, so it opens on nothing but itself and the user's terminal is still theirs, untouched, when Virtui exits. A Provider Workspace declares the shell it offers for each Resource in its own words, in the shell a Resource of that kind actually has: Docker's containers get `/bin/sh`, because the image may hold nothing else; an Incus instance is a whole system, so it gets root's login shell. A Resource its Provider declares no shell for offers the operation nowhere rather than failing when asked. Only a shell that never started counts as failing: one that ran exits with the status of the last command the user typed into it, which is theirs to read rather than Virtui's to complain about.
_Avoid_: Exec, attach, terminal session, embedded terminal
