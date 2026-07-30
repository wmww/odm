# API migration guides

One file per version hop: `vN.md` covers migrating v(N-1) → vN. Bullets
only — a mechanical before → after per breaking change, each snippet
doctested under its respective version. Guides freeze once written (typo
fixes ok).

`odm docs changes <from> <to>` serves the concatenated path, so an agent
updating a file gets exactly the deltas it needs.

None exist yet: there are no stamped versions, and the unstable channel
breaks without migration guides.
