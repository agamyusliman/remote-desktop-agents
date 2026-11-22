# Screenshots

The desktop app handles pairing and permissions; the PWA is the remote itself.

![Desktop app and nine mobile screens](assets/screenshot-grid.png)

## Desktop app

| Connect + pairing QR | Pairing code close-up |
|---|---|
| ![Desktop connect tab](assets/screenshots/desktop-connect.png) | ![Pairing QR and code](assets/screenshots/desktop-qr-code.png) |

## Mobile — 3 × 3

Every capture is 1170 × 2532 and rendered at a fixed width here, so the grid
stays even no matter how the table is laid out.

<table>
  <tr>
    <td align="center" width="33%"><img src="assets/screenshots/mobile-home.png" width="240" alt="Home"><br><sub>Home</sub></td>
    <td align="center" width="33%"><img src="assets/screenshots/mobile-projects.png" width="240" alt="Projects"><br><sub>Projects</sub></td>
    <td align="center" width="33%"><img src="assets/screenshots/mobile-sessions.png" width="240" alt="Sessions"><br><sub>Sessions</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="assets/screenshots/mobile-files.png" width="240" alt="Files"><br><sub>Files</sub></td>
    <td align="center"><img src="assets/screenshots/mobile-remote.png" width="240" alt="Remote"><br><sub>Remote (CLI)</sub></td>
    <td align="center"><img src="assets/screenshots/mobile-split.png" width="240" alt="Split"><br><sub>Split</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="assets/screenshots/mobile-markdown.png" width="240" alt="Markdown preview"><br><sub>Markdown preview</sub></td>
    <td align="center"><img src="assets/screenshots/mobile-code.png" width="240" alt="Code viewer"><br><sub>Code viewer</sub></td>
    <td align="center"><img src="assets/screenshots/mobile-settings.png" width="240" alt="Settings"><br><sub>Settings</sub></td>
  </tr>
</table>

## Notes on these captures

- Device ID, hostname, local IP and paths are placeholders; the relay address is
  `relay.example.com`. A real deployment shows your own values.
- The file tree is scoped to this repository, not to the author's home directory.
- The screen area in Split shows a neutral demo page, not the machine that
  produced these captures.
- The pairing code is single-use and expired when you read this.
- Split is stacked because the phone is in portrait; in landscape it becomes
  screen left, terminal right.
- Source files are coloured with highlight.js (vendored in `public/vendor`).
