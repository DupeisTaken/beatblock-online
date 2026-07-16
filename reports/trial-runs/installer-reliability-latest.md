# Installer reliability and Lovely recovery trial

**Result:** PASS for transaction, diagnostics, payload recovery, administrator installation, and native OBS source registration
**Date:** 2026-07-16 (Windows x64)  
**Selected target:** `E:\beatblock-online\.reference\Beatblock`
**Installer SHA-256:** `6c71dab1ca15b1b0d0819091edbd7ebfcacb26d4131b3cc82ac5f90b1fc91bbc`

## Observed workflow

1. Repair reproduced the reported post-UAC `administrator helper failed with exit code 1` result without changing the working installation.
2. The helper result channel was corrected to retain and return its terminal error. The next run exposed the actual Windows diagnostic: the firewall application path was invalid.
3. The runtime path stored by the legacy manifest contained a forward slash. Win32 file access accepts that spelling, but `netsh` rejects it in `program=`. The firewall boundary now normalizes the path to backslashes, and its regression test uses a deliberately mixed path.
4. The normalized command reached the expected Windows `requires elevation (Run as administrator)` response. The elevation classifier now recognizes that exact wording in addition to access-denied and administrator-required errors.
5. Firewall reconciliation occurs once inside the installation transaction. It tolerates an absent old rule, applies the selected Private/Public profile in the elevated pass, and preserves that profile for Repair. OBS installation is also not repeated after the elevated helper returns.
6. The Components table now enables **Repair Required Components** when a required row is in the yellow Attention state, including a missing firewall rule.
7. The helper writes a terminal error record before its GUI-subsystem process exits. A direct invalid-target trial returned the selected missing folder and `does not contain Beatblock.exe`, rather than a bare exit code.
8. The existing `.reference` installation remains usable after every failed or dismissed privilege attempt. Its previous Lovely recovery still loads `bbt/dashboard_model.lua`, reaches `Initialization complete`, opens Online, and terminates the hidden runtime through Exit Online.

## Privileged physical gate

The UAC prompt was accepted manually. The elevated helper completed, the program-scoped firewall rule was written with the normalized runtime path, and the visible installer received its terminal success record before reporting completion.

## OBS source recovery evidence

The previous release embedded a zero-byte `beatblock-together-obs.dll`, so selecting the checkbox could not install a loadable source. The reviewed OBS 32.0.4 x64 module is now 66,081 bytes and exports `obs_module_load`, `obs_module_ver`, and `obs_module_set_pointer`. The installer rejects an invalid embedded payload before installation, places the module and locale in OBS's ProgramData plugin layout, and verifies their hashes after elevation.

The physically installed module SHA-256 was `dd5052508268fef635a45ee66ef6326c3b5feb29c98aca32421e6c74d9bdbbcf`. After restarting OBS Studio 32.0.4, its current log contained:

```text
[Beatblock Together] OBS sources registered
Loaded Modules:
  beatblock-together-obs.dll
```

The OBS Add Source menu visibly listed **Beatblock Together Player Stream** and **Beatblock Together Shared Audio**. The player source's runtime frame path was also corrected to include `data\render-streams`, matching the renderer publisher. Shared Audio is a registered contract only and does not emit audio in this alpha.

## Automated gates

- 27 Rust unit tests passed with `installer-ui`, including the real embedded OBS module/export check, invalid OBS payload rejection, recommended plugin layout, verified component diagnostics, exact Windows elevation wording, normalized firewall arguments, legacy firewall-profile migration, arbitrary Unicode targets, missing-dashboard migration, payload conformance, monotonic progress, and rollback.
- One dashboard-model test, four protocol-v2 runtime tests, one Beatblock Lua compilation test, and four release stress tests passed.
- Four TypeScript scoring/protocol tests passed.
- Both mod distributions packaged; 13 Lovely signatures, three GameManager hooks, 18 in-game commands, and both ZIP payloads validated.
- The helper terminal-status trial returned the full underlying validation error.
- No Beatblock, runtime, installer, Cargo, or Rust compiler test process remained after cleanup.

## Lovely crash recovery evidence

The earlier Lovely panic was caused by a declared source missing from the installed preloaded sources:

```text
Module source "bbt/dashboard_model.lua" not found in preloaded sources
```

The centralized player and renderer payload inventory includes that module, and conformance tests compare every Lovely `source` declaration with both embedded payloads. The verified Lovely log for the selected target contained:

```text
Lovely 0.9.0
Game directory is at "E:\\beatblock-online\\.reference\\Beatblock"
Initialization complete in 9ms
```
