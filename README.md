# Splice

Splice is an MCP-first video editor for macOS. A headless Rust core owns each project, and the CLI,
MCP server, and native viewer use the same editing and undo history.

Splice 0.1 includes:

- Journaled `.splice` projects, frame-accurate edits, and undo for individual edits or whole agent sessions.
- An MCP server over stdio and a native viewer with playback, scrubbing, and frame stepping.
- AVFoundation export to H.264 or HEVC and PNG frame grabs from the same composition.
- Optional screen capture that creates editable takes with separate audio, camera, and input-event components.

## Build and try the core

Install Rust through `rustup`, then clone with the pinned PXUI submodule. The repository's toolchain
file selects the Rust version.

```sh
git clone --recurse-submodules https://github.com/olegpustovit/splice.git
cd splice
make check
cargo run -p splice_cli -- new demo.splice
cargo run -p splice_cli -- describe clip.insert
cargo run -p splice_cli -- check demo.splice
```

The editing core and stdio MCP server run on Linux. Video decoding, the viewer, export, and capture
use native macOS frameworks. On macOS, the signed `make dev` and `make install` targets use the
project's Developer ID signing setup. See [the build guide](docs/LOOP.md) for those prerequisites.

## Use an installed build

Run `splice mcp` to connect an MCP client, or `splice open demo.splice` to view a project. To create
an editable take, grant the requested macOS permissions and record into a `.splice` package:

```sh
splice permissions --request all
splice record demo.splice --display main --duration 3 --mic
```

See the [MCP contract](docs/MCP.md), [project format and CLI reference](docs/FORMAT.md), and
[milestone board](PLAN.md) for details. `main` is the default branch. Ongoing work goes to `dev`.

Splice is available under the [MIT license](LICENSE).
