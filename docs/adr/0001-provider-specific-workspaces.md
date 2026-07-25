# Use provider-specific workspaces

Virtui will place each provider in a tailored workspace within a shared application shell rather than mixing every provider's resources into one universal model. Docker, Incus, and future providers overlap in some operations but have meaningfully different resource types and capabilities; preserving those native models avoids a lowest-common-denominator interface while still allowing shared interaction patterns such as navigation, logs, filtering, and lifecycle actions.
