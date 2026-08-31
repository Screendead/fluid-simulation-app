# CLAUDE.md

## 1. Purpose

This file gives Claude Code the rules for work in this repository. It holds
the rules, not the project state. `HANDOFF.md` holds the state. **If this file
and HANDOFF disagree about state, HANDOFF wins.**

## 2. What this repository is

A box of liquid you hold. The app runs a fluid simulation on the GPU, driven
by the phone's motion sensors, and renders it as water, as particles, or
coloured by a field such as density, pressure, temperature or velocity. The
source is Rust; the product is an iOS app.

The oracle is real-time performance and efficiency on the reference device
(section 5). Every design choice answers to it. Jack's rule, 2026-08-30:

> Never sacrifice performance for code that is easier to read. Never
> sacrifice easy-to-read code for performance that can be gained while
> retaining easy-to-read code.

State, in one line: M0, the toolchain slice, is in progress. Read
`HANDOFF.md` for the live state and the next task.

## 3. Repository map

| Path | What it holds |
|---|---|
| `crates/fluid-core/` | The simulation and rendering core. Pure Rust. No platform type crosses its boundary. |
| `crates/fluid-ffi/` | The C ABI the iOS shell links, built as a static library. `include/fluid_ffi.h` is generated. |
| `platforms/ios/` | The Swift shell. XcodeGen builds the project from `project.yml`. |
| `scripts/` | The gate, and the build and run scripts. |
| `docs/design/` | The decision slate and the per-milestone design records. |
| `.github/workflows/` | CI. The authority on what the gate runs. |

## 4. Commands

Run these from the repository root. `rust-toolchain.toml` pins the toolchain.

```sh
scripts/gate.sh        # fmt, clippy, tests, header drift, iOS static library
scripts/run-ios.sh     # build, sign, install and launch on the reference device
scripts/run-sim.sh     # build and launch in the simulator: a link-and-launch check only
```

CI runs the gate on every push to master and every pull request, then
builds the iOS app unsigned. Topic-branch pushes do not run CI: the
macOS runner bills minutes at 10x, and the local gate is the same
script. Run the gate before you push. Three notes:

- The gate is strict. One clippy warning or one unformatted file fails it.
- The phone is the default target for every run: measurement, validation,
  eyeballing. Jack's call, 2026-08-30. A simulator build proves the shell
  compiles, links and launches, and is the fallback when the phone is
  away; it has no motion sensors and proves nothing else.
- Xcode is a toolchain here, not an editor. Everything builds from the
  terminal or from VS Code. Open Xcode for the Metal debugger and GPU frame
  capture, and for nothing else.

## 5. Reference device

| Fact | Value |
|---|---|
| Device | Jack's iPhone 13 Pro Max (`iPhone14,3`, A15, 120 Hz display) |
| OS | iOS 26.3.1. **Jack will not upgrade it.** |
| `devicectl` identifier | `1B834EFE-A784-5F98-9B7A-CF6D83E2123A` |
| Deployment target | iOS 17.0 |
| Signing | Personal Team `8N575942RT`, automatic signing, free account |

Every performance number is measured on this device. A number without a
device and a date is not a measurement. Re-measure; do not trust a recorded
number.

## 6. Documents and authority

| Document | What it governs |
|---|---|
| `HANDOFF.md` | Project state, the roadmap, and the next task |
| `docs/design/decisions.md` | The D-number decision slate. Every record binds the code. |
| the other files in `docs/design/` | The per-milestone design records |
| `REVIEW.md` | The criteria for every code review |

A design record outranks the code. Amend a record by an explicit edit, never
silently.

Where two authorities require different things and no rank settles it, that
is a peer collision. Stop and bring it to Jack, in plain English: name the
two, quote each, state what each would have you do, give every option with
its concrete cost, and recommend one. Never settle one alone. This holds even
when the work is already done; say so.

## 7. Code standards

### Language boundaries

- `fluid-core` holds the simulation and the rendering. It has no platform
  type and no platform dependency. Sensor input enters as `MotionSample`.
- Shaders are WGSL, wgpu's shading language. No Metal Shading Language,
  no GLSL.
- The platform shell does what only the platform can do: sensors, the
  drawing surface, permissions, haptics, the app lifecycle. Nothing else.
- The simulation reads the sensors of the device that runs it. Sensor data
  never crosses devices: no forwarding phone to laptop or laptop to phone,
  in any variant, ever. Jack's rule, 2026-08-30.

### Performance

- The rule in section 2 governs every trade. Expect data-oriented layout,
  GPU-resident state, and no allocation in the per-frame path.
- A change to a hot path carries a measurement on the reference device:
  before, after, and how it was taken.
- Idle costs nothing. A still or backgrounded phone runs no simulation
  step.

### Comments

Jack's rule, 2026-08-30: a comment marks intention where it is unclear. It
never redescribes the code. When intention is unclear from the code itself,
that is a code smell: rethink the code first. A comment is acceptable only
when that is not possible.

A doc comment adds a fact beyond the symbol's name, or it is omitted.

### No untested and unexercised code

Code that no test asserts on **and** that no run reaches must not enter the
repository. Delete it. Two words carry the rule, and they are not the same
test:

- **Exercised** — a normal run of the app or a script reaches the code.
- **Tested** — a test asserts on what the code does.

| Exercised | Tested | Verdict |
|---|---|---|
| yes | yes | Good. |
| yes | no | Permitted, with caution. Prefer to add the test. |
| no | yes | Permitted. The code needs a concrete plan to get a caller; record it in the design record that asks for the code. |
| no | no | **Banned. Delete it.** |

The rule binds a whole symbol and each of its parts: an unread field, an
unused parameter, an unreachable branch, a constant with no reader.

Two consequences: do not write scaffolding for a later milestone, and add a
dependency when its first caller arrives, not before.

### Refactoring

Standing authorization, Jack, 2026-08-30: "Code pruning, restructuring,
refactoring are always acceptable." Prefer deletion over annotation. Add an
abstraction only to remove a repeated idea, never to save lines. Match the
idiom of the file around you.

### Generated files

Do not edit these by hand.

| File | Generator |
|---|---|
| `crates/fluid-ffi/include/fluid_ffi.h` | `cbindgen --config crates/fluid-ffi/cbindgen.toml --output crates/fluid-ffi/include/fluid_ffi.h crates/fluid-ffi`; the gate rejects drift |
| `platforms/ios/FluidApp.xcodeproj`, `platforms/ios/Sources/Info.plist` | `xcodegen generate` in `platforms/ios/`; ignored by git |

## 8. Workflow

- Through the close of M0, commit straight to master and push; no pull
  requests. Jack's call, 2026-08-30, "while we're getting set up".
- From M1, branch off master with a topic slug, for example `m1-surface`,
  and do not commit to master. Commit to the topic branch and push it
  without asking. Ask before you open a pull request: that call is Jack's.
  So is the merge.
- Commit as you go. Each commit leaves every component working.
- Run the gate before every push. CI must pass before a merge.
- A decision with one viable option does not stop and wait. Take it, record
  it in HANDOFF or the design record with the rejected options and their
  costs, and continue. Jack can overturn it; silence lets it stand.
- Write short, imperative, jargonless commit messages.
- Hand-pick the model for every subagent, workflow agents included: set
  the model on every `agent()` call. Jack's directive, 2026-08-30. The
  cheapest model does mechanical checks — a quote exists, a file
  matches. A mid model does standard verification and mechanical edits.
  Only design, review lenses and physics judgment get the top model, and
  mechanical stages run at low effort.
- A multi-agent refactor gets an independent adversarial review before
  merge. The reviewer starts with fresh context and reads the diff and the
  repository, never the author's plan. Every finding cites `file:line` and
  quotes the text.
- Update `HANDOFF.md` in the same commit that closes a milestone or a task.

These stop and wait for Jack: a pull request; a merge; a peer collision
(section 6); a change to section 7 or 8 of this file or to `REVIEW.md`; a
dependency the decision slate does not already name; anything that costs
money.

## 9. Prose style

Repository prose (`HANDOFF.md`, `docs/`, this file) follows a short form of
Simplified Technical English: short sentences with one idea each, active
voice, the imperative for procedures, one word for one meaning, and a table or
a list over dense prose. `README.md` is the front door and is exempt. Code
comments and commit messages are exempt.
