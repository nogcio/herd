# COSMIC and Linux integration

## Desktop integration

The applet uses `cosmic::applet::run`, native `Core::applet` controls,
autosizing, popup settings, popup close events, and native theme surfaces. This
follows the
[COSMIC applet model](https://github.com/pop-os/cosmic-applet-template), using
the exact libcosmic source selected by `Cargo.lock` rather than assuming latest
API compatibility. No GTK/Qt theme override, generated palette edit, X11 tray
emulation, custom Wayland shell, or desktop-session restart is required.

Both panel orientations use a native symbolic icon button, with warning/download
variants for errors and active transfers. Model details remain in the
same popup. Dimensions are logical UI units under COSMIC scaling, not physical
pixels. Native controls supply keyboard/focus handling, and libcosmic
accessibility support is enabled. Icon-only actions have accessible names
without visible tip text. Physical keyboard/screen-reader/scaling acceptance
remains a separate native check.

Gather is the adopted application icon, supplied by project assets and package
integration rather than a global theme change. Its
[provenance and license](icons.md) remain explicit. Updating source assets does
not update an already installed package or prove live launcher/panel rendering.

The desktop entry and AppStream component share
`io.github.nogcio.Herd.Cosmic`, matching the application ID and desktop
filename. The application is visible in the COSMIC launcher. A separate hidden
`io.github.nogcio.Herd.Cosmic.Applet.desktop` entry is marked
`X-CosmicApplet=true`. Its homepage/source/issue URLs refer to this
application's GitHub repository. A future identity change requires coordinated
registration and panel references, not a silent rename of installed files.

## Paths and installation

Use [the Arch package](installation.md) for installation and updates. Pacman
owns application files; no parallel user-local installer is maintained.

- Hosting preferences default to
  `XDG_CONFIG_HOME/herd/server.json`; the native preset path is
  explicit. Process Compose owns private Unix IPC and the router's lifetime in
  an independent user scope. The backend requires the session bus and systemd
  user manager; no system-wide service or root access is used.
- Artwork cache honors `XDG_CACHE_HOME`, defaulting to
  `~/.cache/herd`. Atomic thumbnail writes use standard
  temporary files in the destination directory.
- Desktop/AppStream files are packaged under `/usr/share`. Backend activation
  uses `/usr/share/dbus-1/services` and `/usr/lib/systemd/user`, with an
  absolute `/usr/bin/herd-service` executable. No package-owned system service
  or enable/start/restart hook is provided.
- Previous user-local registrations can override packaged files. The package
  does not modify home directories; review and authorize any cleanup separately.
- Model profiles, personal configuration, downloaded artwork, local logs, and
  model weights are not application installation assets or Git source content.

Build/check commands do not use sudo, write system units, start inference,
enable autostart, or restart a graphical session. Explicit `server-up` starts
a loopback router listener and private Unix management socket. Host executable
installation is a separate reviewed action. This application is not a sandbox
for the configured native runtime.

## Scope and acceptance

The implementation targets Linux/COSMIC. There is no GPU utilization/memory
monitoring dependency. Native llama.cpp GPU/context parameters and
vendor-neutral COSMIC rendering remain independent supported concerns. Router
state is not process-idle or inference-quality detection. The GUI has embedded
English and Russian Fluent resources with desktop-locale negotiation and English
fallback.
Technical backend diagnostics and CLI machine output are not translated. See
[UI conventions](ui.md).

Desktop/AppStream syntax, headless widget construction, Cargo checks, and the
[manual acceptance route](testing.md#native-acceptance) establish
different properties. Metadata validation does not certify COSMIC Store
publication, live catalog visibility, every distro, all screen readers, or
unattended model safety.
