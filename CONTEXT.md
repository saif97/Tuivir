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

**Pane**:
A focusable region of the application shell, such as the Provider selector, a Resource Panel, or the Details pane. Use _Panel_ only for a provider-defined Resource Panel.
_Avoid_: Area, section, focus region

**Resource**:
One selectable native thing managed by a Provider, such as a container, image, volume, network, or instance. A Resource has a Resource State only when its kind has a lifecycle state.
_Avoid_: Item, entity, provider object

**Resource Panel**:
A provider-defined Pane containing one kind of Resource, such as Docker Containers or Images.
_Avoid_: Resource pane, resource list

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

**Resource Shell Session**:
An ongoing command shell attached to one Resource through a Detail View. It remains active when the user navigates elsewhere in Virtui and is ended explicitly or when Virtui exits.
_Avoid_: Interactive Detail Session, shell panel, terminal tab

**Resource State**:
What a Provider reported a stateful Resource to be doing at the last refresh, in one vocabulary shared by every Provider: running, stopped, paused, transitioning, broken, or unknown. Each Provider Workspace maps its own status words into it, and an invoked Command carries it so Virtui never asks a Provider CLI for what it already knows. A stateless Resource has no Resource State; absence is not _unknown_. Only _stopped_ is positively determined; every other state, unknown included, means "not settled and stopped", so a Command that must treat those differently fails safe.
_Avoid_: Status, run state, power state, phase
