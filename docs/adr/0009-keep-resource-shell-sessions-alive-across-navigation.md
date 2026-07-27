# Keep Resource Shell Sessions alive across navigation

Virtui will keep a Resource Shell Session active when the user changes Resources or Provider Workspaces, restore it when its Resource is revisited, and expose persistent indicators for hidden sessions. A session ends only through an explicit close or when Virtui exits; normal Quit requires confirmation while sessions are active, but emergency Quit remains immediate. This deliberately accepts bounded background resource use so navigation never destroys an interactive task or silently restarts it.
