# Testing the review fixes

Branch `fix/review-findings`, on top of v0.1.0 (`6a53ddb`): the review fixes (`253cbf1`), then
timelines on particle instance overrides. It stays off
`main` until it has run on the Linux machine. apolline-wall's branch of the same name pins this
commit; its [docs/TESTING.md](https://github.com/Pisosenie/apolline-wall/blob/fix/review-findings/docs/TESTING.md)
covers the daemon.

Checked so far: `cargo test` (191 tests) and `cargo clippy --all-targets` pass on the Mac, and
apolline-wall's Linux clippy builds against this branch. Nothing here has drawn on a real GPU.

## Set up

```bash
cd ~/src/apolline-scene && git fetch && git switch fix/review-findings
cargo test && cargo clippy --all-targets -- -D warnings
```

Below, `W` is the Workshop folder and `A` Wallpaper Engine's `assets` folder:

```bash
W=~/.local/share/Steam/steamapps/workshop/content/431960
A=~/.local/share/Steam/steamapps/common/wallpaper_engine/assets
```

## Whole-library checks

Run these first on `main`, then on the branch, and compare.

1. `cargo run --release --example survey -- $W $A`: how many scenes prepare. The branch should
   prepare as many as `main` or more. One may now prepare that `main` refused with "the scripts
   ran too long" (item 5 below). Any scene the branch refuses that `main` prepared is a
   regression; `cargo run --release --example blockers -- $W $A` says why.
2. `tools/render-all.sh $W $A`: every scene that prepares draws a frame without a shader
   failing. Items 3 and 4 below change shader text.
3. `bash tools/gles-check.sh`.

## What changed, and how to see each

1. **Text a script changes counts its effect buffers.** A text layer with effects that a script
   retexts (clocks, counters) now reserves its effect buffers at the new size, and if GL refuses
   the new words the old ones go back in whole. Check: a clock scene keeps drawing its text and
   its effects (glow, shadow) for a few minutes, with no budget error in the log.
2. **A child particle system's warm-up counts its leader's particles.** A child following a big
   parent (`eventfollow`, `eventspawn`, `eventdeath`) warms up for less time instead of stalling
   the first frame for seconds, and a leader's copy is no longer warmed up only to be thrown
   away. Check: scenes with sparks trailing stars or bursts on death show their first frame
   sooner than on `main`, and look the same after a few seconds.
3. **`max(0, 1)`-style calls are turned once.** Before, a `max` or `min` with two bare numbers
   was flipped back and forth until the 256-turn limit, and later calls in the shader were never
   turned. Check: item 2 of the whole-library checks.
4. **Hex numbers stay whole.** `0x1F`, `0xAF` and the like were read as fractions because of the
   `F`, so an `int == 0x1F` comparison was made a float one and failed to compile. Check: item 2
   of the whole-library checks.
5. **The scripts' first frame gets the loading time.** Every `init()` runs in the first frame;
   it had 50 ms and now has the 2 s loading budget. Check: a scene with heavy scripts that `main`
   refused with "the scripts ran too long" now prepares.
6. **Skeletons of an unknown version are refused.** Puppet skeletons other than `MDLS0001` to
   `MDLS0003` are refused, as STATUS.md already claimed. Check: every puppet-warp scene that
   drew on `main` still prepares. If one is now refused with "a puppet skeleton version", see
   which version it has with `python3 -I tools/pkg.py scene.pkg <model>.mdl | grep -ao 'MDLS[0-9]*'`
   and tell me.
7. **Timelines on particle instance overrides.** An `animation` on `instanceoverride.alpha`,
   `count`, `lifetime`, `rate`, `size` or `speed` now plays, the way a script on one already did.
   This alone refused 2998757800 and 3233141951. Check: both prepare (`survey` counts two more
   than `main`), and their particles fade, thin out or speed up over time the way the Workshop
   previews show.
8. **Decoded textures may take 1 GiB, not 512 MiB.** This is the same as the GPU budget, so a
   scene can't be refused for its decoded textures and then fit on the GPU anyway. It blocked
   the Lain scene 2594686226 and 2388299037, though 2388299037 also needs user textures.
   - **Check:**
     1. 2594686226 prepares, or is now refused only by the GPU budget: "scene GPU texture
        allocations exceed 1 GiB" at `Renderer::new`.
     2. If it draws, it matches its preview.
     3. While it loads, the daemon's memory goes up by up to about 1 GiB and comes back down
        once it shows.
