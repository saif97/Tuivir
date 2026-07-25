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
