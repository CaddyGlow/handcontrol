# Third-Party Notices

HandControl is distributed under the MIT License. The project also incorporates
third-party components that carry their own, compatible licenses. This document
collects attribution and licensing requirements for those dependencies.

## Termux Terminal Components (Apache-2.0)
- Modules: `terminal-view`, `terminal-emulator` (vendored from
  [termux/termux-app](https://github.com/termux/termux-app))
- Original upstream project:
  [jackpal/Android-Terminal-Emulator](https://github.com/jackpal/Android-Terminal-Emulator)
- License: Apache License 2.0 (see `licenses/Apache-2.0-Termux-Terminal.txt`)
- HandControl modifications: made `TerminalSession` extensible and added a Kotlin
  `RemoteTerminalSession` bridge to stream remote PTY data into the Termux emulator
  (documented in `docs/android-terminal-integration-plan.md`)

Future third-party additions should extend this file with the component name,
upstream URL, license, and a brief summary of local changes.
