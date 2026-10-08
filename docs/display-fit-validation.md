# Post-release display fit correction

This correction is included in Windows 0.1.1. The immutable Windows 0.1.0
installer and its tagged source archive retain the earlier rendering behavior.

Native ARM64 macOS tests found a missing last column in portrait footage.
Default RGB decoding, scalar RGB decoding and the rendered Y plane all measured
99 rather than 100 white columns. The source's entire 90-by-160 Y plane was
white, so the loss occurred during rendering rather than fixture encoding.
The same loss was reproduced on Windows by rendering with `-cpuflags 0`.

Stage isolation identified the odd-width YUV-to-RGBA conversion: a 101-by-180
intermediate gained a transparent last column, and the subsequent even scale
produced only 99 opaque columns. The renderer now normalizes its initial fit to
even dimensions before RGBA conversion. It retains the existing policy of
rounding to the nearest integer, then flooring to an even size with a minimum
of two pixels. This preserves fractional fits that should round up to an even
integer. Source and proxy inputs follow the same graph.

## Silent local checks

The focused actual-media regression renders every case through the optimized
and scalar FFmpeg paths. It asserts exact centered RGB bounds, scalar rendered
Y-plane bounds, square-pixel source/proxy caches and original/proxy RMS below 3.
The existing geometry cutoff and dimensions remain strict.

| Source display dimensions | Fitted dimensions | Original/proxy RGB RMS |
| --- | --- | ---: |
| Portrait 90 x 160 | 100 x 180 | 0.000 |
| Anamorphic 320 x 90 | 320 x 90 | 0.000 |
| Rotated anamorphic 90 x 320 | 50 x 180 | 0.000 |
| Rotated fractional SAR 90 x 166 | 98 x 180 | 0.000 |

All eight source/proxy cases passed after the final rounding correction. The
fractional fixture uses SAR 83/80 and catches an incorrect direct floor that
would shrink its fitted width to 96. During validation, 43 related renderer,
source-clock, split/envelope and cache tests passed; four library lifecycle/time
tests also passed. The final focused geometry repeat took 5.45 seconds. Media was
generated, encoded and decoded to files or sample data; no speaker playback or
native UI was started.

Preview recipe `program-preview-v2` invalidates old v1 render records. The cache
regression injects a stale v1 recipe and checks that a real replacement render
occurs. The project format is unchanged. The corrected engine at commit
`c641352d5bc22f55a0a19d721d3cb9a207ebf851` passed all 54 active Rust tests on
[Linux](https://github.com/not-dlyrr/mono-cut/actions/runs/37760482289/job/113255410405)
and [macOS ARM64](https://github.com/not-dlyrr/mono-cut/actions/runs/37760482289/job/113255410075).
One explicit performance benchmark remains ignored in ordinary CI. The subsequent
release version metadata does not change this rendering code. Native
Linux/macOS installer playback remains separate work.
