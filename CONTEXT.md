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
