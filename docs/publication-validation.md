# Public release verification

The public repository and Windows 0.1.0 prerelease were verified on 2026-10-08
through unauthenticated HTTP requests. The repository is public, the release
page returns HTTP 200, and all six downloadable asset URLs return HTTP 200.

- [Public repository](https://github.com/not-dlyrr/mono-cut)
- [Windows installer](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.0/mono-cut-0.1.0-windows-x64-setup.exe)
- [Release and corresponding source](https://github.com/not-dlyrr/mono-cut/releases/tag/v0.1.0)
- [Checksums](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.0/SHA256SUMS.txt)

The immutable release tag identifies application commit
`0af14da1dce29164aadb55d565b1e0e5d0252ad4`. Public tagged App source matches the
reviewed local source byte for byte. The downloaded installer, app source ZIP,
BUILD-INFO and checksum manifest were independently rehashed. All six GitHub
server asset digests and sizes match the locally packaged release.

| Asset | Bytes | SHA-256 |
| --- | ---: | --- |
| Windows installer | 34,558,170 | `065fbc50f426bb8304e914345cd3e411da1132337bccbd592898c388c563c348` |
| Application source | 1,558,047 | `9c0da15fa6c7b663a8b28d3f8f35a1e2cf7f3c69a2551dc2e874661857296a01` |
| Media source | 79,579,716 | `f1c9870e2265194e03610d1fcf6cf8eccf3c31e8edbe67cf6772ea966f6bace8` |
| Dependency source | 131,089,481 | `3f09a885d1a1e666c0268200ba73f91dcd78e5c7b3b04ae5cfdab29fa8842b08` |

The two large auxiliary source ZIPs were verified against their GitHub server
digests and public URLs after upload, rather than downloaded a second time. Their
inner sources, hashes and lockfiles were verified locally before publication.
Static installer integrity and the native/UI validation limits are recorded in
[the release record](release-0.1.0.md).

## First CI portability follow-up

The first Linux/macOS runs compiled the interface and application but failed
strict actual-media checks with their system FFmpeg builds. Ubuntu's older media
engine produced short frame counts and an incorrect dissolve midpoint. The newer
Homebrew engine rejected the removed `filter_complex_script` command option.
Those assertions remain strict. Unix development/CI now uses the native source
build recipe for the audited FFmpeg 8.1.1 baseline; arbitrary system FFmpeg major
versions are not a validated substitute. Shell syntax, embedded JavaScript,
workflow structure and early argument/platform guards were checked locally.
An initial native archive-name difference was corrected without changing the
media assertions. Both native Unix media builds then passed. In
[run 37755934520](https://github.com/not-dlyrr/mono-cut/actions/runs/37755934520),
Linux passed the full engine/media suite; macOS passed eight workflow checks but
the portrait-fit check measured a 99-pixel bright footprint where 100 was
expected. Diagnostics proved a rendered edge loss and reproduced it through
scalar rendering on Windows. The post-release source correction normalizes the
initial fitted canvas to even pixels before RGBA conversion and updates the
preview recipe. Exact geometry assertions remain in place and cover both
optimized and scalar rendering, including a fractional aspect ratio.
[Display fit validation](display-fit-validation.md) records the evidence and
silent local results. The corrected source awaits its full cross-platform CI
results. The published Windows package and its audited media baseline remain
unchanged.

CI is distinct from a verified native Linux/macOS installer release. The old
0.1.0 tag/downloads remain intact while the CI preparation follow-up advances
main. Local Windows tests and public download verification are not presented as
cross-platform native playback evidence.

The [tagged Windows job](https://github.com/not-dlyrr/mono-cut/actions/runs/37753520208/job/113232323103)
completed successfully: interface checks and 28 helper tests, all 54 active
engine/media tests, seven open-loader tests, installer build, static package
integrity and artifact upload. The complete tag workflow failed on the original
Unix system-version mismatch described above. The Windows CI installer is a
separate build artifact; it does not replace the public release download or its
checksum. It was checked without launching the editor. Linux's later successful
engine run likewise does not establish native GUI playback or installer behavior.
