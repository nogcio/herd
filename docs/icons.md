# Artwork

Use model/developer logos in model rows, without an added square background.
The panel uses a native symbolic application/status icon, not remote artwork.

## Herd application artwork

Herd's **Gather** icon depicts three separate rounded model volumes on a shared
plinth: a managed collection, not a chat interface or inference engine. It is
original project artwork authored from scratch with OpenAI assistant assistance,
not a model/developer logo or a derivative of third-party icon paths. The SVG
sources are MIT licensed under the project's [license](../LICENSE), with SPDX
license identifiers in each source. No external assets or fonts are required.

The launcher identity is `io.github.nogcio.Herd.Cosmic`; both desktop entries
and AppStream use that installed icon name. Its original scalable SVG source is
`resources/icons/hicolor/scalable/apps/io.github.nogcio.Herd.Cosmic.svg`.
The matching panel identity is `io.github.nogcio.Herd.Cosmic-symbolic`, with
separate optically drawn SVG sources in `resources/icons/hicolor/16x16/apps/`
and `resources/icons/hicolor/24x24/apps/`. These transparent, single-color
drawings use `currentColor`; foreground color belongs to the native renderer,
not an application badge background. The package preserves these hicolor paths
under `/usr/share/icons/`. It does not install an icon theme or theme-update
hook.

The artwork's MIT license does not change the combined application's
`GPL-3.0-only` AppStream project license or dependency obligations. Model
artwork below remains separately owned and licensed.

## Metadata

Add optional presentation comments inside a native model preset section:

```ini
; @icon-url=https://cdn-avatars.huggingface.co/v1/production/uploads/6a7166893cf1b834b1a7dfff/B8aWcllku8aYdub8_vi2A.png
```

`; @icon=` can instead name an installed desktop theme icon, or an absolute
local PNG/JPEG/WebP path. No shell or home-directory expansion is performed. A
resolved theme icon or valid local image takes priority over `icon-url`.
Missing/broken sources fall back to remote artwork if provided, and then to
initials. Local SVGs are not decoded by the raster loader; trusted installed
theme SVGs use COSMIC's native theme renderer.

Presentation comments stay in the native preset file, not a separate icon
registry. File changes are picked up without restarting the UI. Native llama.cpp
ignores these comments; they cannot alter inference settings.

## Loading and cache policy

- Downloads are background tasks, independent of service control and health
  checks.
- Successful artwork is decoded to a small PNG thumbnail (within 128×128, aspect
  ratio preserved), then stored under
  `$XDG_CACHE_HOME/herd/icons/`, or
  `~/.cache/herd/icons/` by default. URL hashes are used as
  filenames.
- A valid disk cache avoids network requests, including after applet restarts.
  Successful entries have no automatic expiry; change the source URL to select
  different artwork. Local/theme changes with an unchanged source need an applet
  reload. Cache writes are atomic and cache-write failures do not prevent
  display.
- In-memory entries avoid reloading on popup opens or periodic status refreshes.
  Failed loads have a ten-minute retry cooldown. Removed metadata discards UI
  state, but does not delete cached files. Stale completions cannot replace a
  changed source.
- HTTPS and certificate verification are mandatory. Only `huggingface.co`,
  `cdn-avatars.huggingface.co`, and `raw.githubusercontent.com` are allowed.
  Initial URLs and every redirect are checked; credentials, query strings,
  fragments, non-standard ports, and other hosts are refused. At most five
  redirects and four concurrent downloads are allowed. No authentication, model
  prompts, or environment proxy credentials are sent. Hosts see ordinary public
  image requests.
- Responses/local files are limited to 512 KiB. Remote SVG/HTML/GIF are not
  supported. MIME type and actual decoded PNG/JPEG/WebP content are checked.
  Decoding is bounded to 2048×2048 pixels and 32 MiB allocation; expensive
  file/codec work runs off the UI executor. Only normalized image bytes reach
  the UI.
- A missing network, unsupported image, absent theme icon, invalid URL, or cache
  error does not become a model/discovery failure. Initials remain the fallback.

## Registered sources

- **Qwen:** the artwork returned by the public
  [Qwen organization API](https://huggingface.co/api/organizations/Qwen/overview):
  `https://cdn-avatars.huggingface.co/v1/production/uploads/6215ca5692c0ecfba9186921/hrRM50-6XcdWgg2AKpENG.jpeg`.
  The response currently contains WebP data; content sniffing is used, not
  suffixes.
- **Flint:** the official developer's logo from the public
  [RecursiveSI organization API](https://huggingface.co/api/organizations/RecursiveSI/overview):
  `https://cdn-avatars.huggingface.co/v1/production/uploads/6a7166893cf1b834b1a7dfff/B8aWcllku8aYdub8_vi2A.png`.
  This is Recursive's logo, not Unsloth's quantizer logo. The separate colored
  `assets/flint-logo.png` model-card asset redirects to a different HF LFS CDN
  with a signed query. That redirect is deliberately blocked by the artwork
  policy; the official organization avatar works without expanding the
  allowlist.

These are third-party logos, not original project artwork. They retain their
owners' rights; they are fetched to the user's cache, not bundled or relicensed
with the application.
