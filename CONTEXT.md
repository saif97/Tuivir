# Tuivir

Tuivir is a terminal environment for inspecting and operating resources managed by local virtualization and container providers.

## Language

**Provider**:
An installed system that owns and operates resources, such as Docker, Incus, or Docker Sandbox. Its domain record keeps its stable identity and display name together with its optional Target Environment and Provider Version.
_Avoid_: Backend, runtime, engine

**Provider Discovery**:
Infrastructure evidence that a Provider is installed, together with an actionable availability failure when the discovery probes themselves find it installed but unavailable. A Provider may deliberately defer Target Environment reachability to Workspace refresh when installation and version can be established without contacting it. Discovery wraps a Provider; it is not a second copy of Provider metadata. An absent Provider CLI produces no discovery.
_Avoid_: Provider state, provider model

**Provider Version**:
Build information reported by an installed Provider. It describes the Provider itself and never identifies the Target Environment that owns the Resources Tuivir operates.
_Avoid_: Target version, environment version

**Provider Workspace**:
The provider-specific view of its native resources and operations within Tuivir.
_Avoid_: Provider tab, unified resource view

**Provider Workspace State**:
Application-owned load and navigation state for one discovered Provider Workspace. It owns the Provider domain record alongside its current snapshot, focus, selection, detail loading, and errors; it does not duplicate Provider metadata.
_Avoid_: Provider Discovery, Provider

**Active Workspace**:
The single provider workspace currently visible and being refreshed. Inactive workspaces remain idle.
_Avoid_: Selected backend, current tab

**Pane**:
A focusable region of the application shell, such as the Provider selector, a Resource Panel, or the Details pane. Use _Panel_ only for a provider-defined Resource Panel.
_Avoid_: Area, section, focus region

**Pane Boundary**:
The movable edge between two Panes, held as the share of the width or height it leaves the first of them rather than as a column or row count — so a terminal that changes size keeps the split the user chose. It is moved by dragging the borders that draw it or by its own Commands, and it holds itself inside a range that leaves both Panes usable. One share serves the whole Tuivir run and every Provider Workspace in it. A completed resize records that preference in Tuivir's XDG state directory; a small terminal may temporarily clamp the drawn width without changing the preference.
_Avoid_: Divider, splitter, split ratio

**Resource**:
One selectable native thing managed by a Provider, such as a container, image, volume, network, or instance. A Resource has a Resource State only when its kind has a lifecycle state.
_Avoid_: Item, entity, provider object

**Resource Panel**:
A provider-defined Pane containing one kind of Resource, such as Docker Containers or Images.
_Avoid_: Resource pane, resource list

**Volume**:
An independently managed storage Resource, such as a Docker volume or an Incus custom storage volume. Storage owned as part of another Resource, a host path mounted into a Docker Sandbox, and a volume snapshot are not Volumes in Tuivir.
_Avoid_: Disk, mount, instance storage

**Target Environment**:
The environment already selected through a Provider's CLI configuration, such as a Docker context or an Incus remote and project. Some Providers do not select one.
_Avoid_: Cluster, server, connection

**Command**:
A registered user intention that Tuivir can invoke within its Command Scope.
_Avoid_: Action, handler

**Running Resource Command**:
An invoked Command whose Provider work for one Resource has not completed. It belongs to that Resource and remains distinct from the Resource State last reported by its Provider.
_Avoid_: Transitioning Resource, pending state

**Command Scope**:
The structural part of the interface in which a Command may be invoked, such as a focused panel or a Provider Workspace's resource view. Mutable resource state does not change a Command's scope.
_Avoid_: Context, condition

**Keybinding**:
An ordered association between one or more key combinations and a Command. The first combination is the preferred inline hint when the interface has room to show only one.
_Avoid_: Shortcut, hotkey

**Detail View Tab**:
One selectable view of a Resource in the Details pane, either supplied by Tuivir or declared and named by its Provider Workspace — Docker's Logs, Stats, and Inspect; Incus's Info, Config, and Console Log. Only the provider-backed tab on screen is ever loaded; snapshot-backed content comes from the current Workspace Snapshot, while a Resource Shell Session continues independently of which tab is visible.
_Avoid_: Detail View, pane, inspector, log view

**Resource Shell Session**:
An ongoing interactive command shell running inside exactly one Resource and presented through its Shell Detail View Tab. At most one runs for a Resource; the same session can be shown inside the Details pane or enlarged to fill Tuivir, continues while the user navigates elsewhere, and ends when its process exits, its Resource disappears, or Tuivir exits.
_Avoid_: Interactive Shell, attached shell, shell panel, terminal tab

**Resource State**:
What a Provider reported a stateful Resource to be doing at the last refresh, in one vocabulary shared by every Provider: running, stopped, paused, transitioning, broken, or unknown. Each Provider Workspace maps its own status words into it, and an invoked lifecycle Command carries it so Tuivir never asks a Provider CLI for what it already knows. A stateless Resource has no Resource State; absence is not _unknown_. Only _stopped_ is positively determined; every other state, unknown included, means "not settled and stopped", so a Command that must treat those differently fails safe.
_Avoid_: Status, run state, power state, phase
