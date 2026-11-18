# Apple platform bridge

`native/platform.h` defines the ABI. Each Apple service uses an ARC Objective-C shim
and a safe Rust module in this crate. The build compiles the shims only for macOS.
Use fixed-width records for data crossing the ABI and keep the C and Rust layout
assertions in sync. A fallible native call transfers its optional error string to Rust.

`media::probe` reads the local asset duration, display dimensions, nominal video frame
rate, codec, and audio track metadata through AVFoundation. Call it on a worker if the
UI needs to remain responsive during the probe.

The native event ring copies records into fixed storage. The consumer peeks into its
own reusable buffer and acknowledges the records it has used. Full rings drop new
records and count those drops. A wake callback runs after the empty-to-ready
transition and receives only a scalar token.

`fixtures/media/probe.mov` is a one-second synthetic color ramp made with AVAssetWriter
at 96 × 64 and 30 fps. `stereo.wav` contains 800 silent stereo PCM frames at 8 kHz.
