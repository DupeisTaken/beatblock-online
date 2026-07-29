# Beatblock compatibility

## Current release

GitHub Release titles include the Beatblock version exercised by that Online
release:

| Beatblock Online | Online protocol | Tested Beatblock build            | Newer Beatblock builds   |
| ---------------- | --------------: | --------------------------------- | ------------------------ |
| `0.3.0-beta.4`   |               3 | `1.7.1a (Early Access)[d40b7083]` | Accepted, but unverified |

The release title is **v0.3.0-beta.4 for Beatblock 1.7.1a+**. The `+` means the
installer accepts later structurally valid Beatblock releases; it does not mean
those releases were tested. `1.7.1a (Early Access)[d40b7083]` is the only
verified baseline for this Online release.

The Git tag remains the machine-readable semver tag `v0.3.0-beta.4`. Keeping
human compatibility text in the GitHub Release title preserves update checks,
package-version verification, and normal Git tooling.

## Installation policy

The installer validates the Beatblock folder structure, the injection method,
and Beatblock Online's own payloads. It does not reject a new upstream release
because `Beatblock.exe` changed. This means an already downloaded installer can
normally install after Beatblock updates without a new Beatblock Online build.

Exact Beatblock identity becomes available after the game starts. Beatblock
draws a version such as `1.7.1a (Early Access)[d40b7083]` in the top-right
corner. The injected adapter sends that complete value to the local runtime,
which uses the bracketed upstream build token (`d40b7083`) as the primary
identity.

If a future game release changes the label format, the runtime deterministically
hashes the complete displayed version instead. If the game stops exposing the
version entirely, it falls back to a digest of the installed code-bearing game
files. Neither fallback needs a per-release registry.

This identity is an interoperability check for honest clients, not anti-cheat
or cryptographic remote attestation. A locally modified game or adapter can lie
about itself. Chart hashing, score validity checks, room authentication, and
normal tournament procedures remain separate controls.

## Same-build rooms

Rooms require the exact same Beatblock build by default. The build identity is
included in the password-authenticated room handshake, before a participant is
added to the roster. A mismatch receives an error showing both displayed
versions and shortened build IDs.

This default prevents players with different bundled chart data, judgement
windows, or upstream gameplay behavior from competing in the same room.

The host can select **Allow Any** while creating a casual room. A strict room
can also be relaxed before a race from **Settings → Same Build**. Relaxing is
one-way for that room: create a new room to restore strict matching. This
avoids an in-flight relaxed connection crossing into a newly strict roster.

## When a new Beatblock release breaks Online

Newer releases are accepted but unverified. An upstream change can still move
an injection point, rename a required file, or change gameplay behavior.

1. Confirm the complete version and bracketed build token in Beatblock's
   top-right corner.
2. Retry with the newest Beatblock Online release.
3. Save a sanitized installer/runtime log. Do not upload Beatblock executables
   or game archives.
4. File the dedicated
   [Installer incompatible with latest Beatblock release](https://github.com/DupeisTaken/beatblock-online/issues/new?template=beatblock_compatibility.yml)
   report.

That issue category records the failure stage, complete displayed version, and
exact bracketed upstream build token so maintainers can schedule and reproduce
an adapter update. The tested baseline can advance in the next Online release,
but existing installers continue accepting structurally valid later builds
unless an actual upstream break requires new code.
