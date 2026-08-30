# Review criteria

Rules for every code review of this repository. A real defect outranks a
style point.

## Severity

- **Blocker:** a real defect. Examples: wrong behavior; a broken invariant;
  a test that cannot fail; code that is neither exercised nor tested
  (CLAUDE.md section 7); a platform type or dependency inside `fluid-core`;
  a shader that is not WGSL; a hot-path change with no measurement on the
  reference device; a hand edit to a generated file; an allocation added to
  the per-frame path.
- **Advisory:** a humanness finding from the list below, a style point, a
  nit. Cap nits at five per review; keep the most useful ones.

Every finding must cite `file:line` and quote the text it concerns. Drop a
finding that cannot.

## Humanness

A humanness finding marks code or prose that reads as machine output, not as
the work of a careful person. Flag these seven. They are advisory.

1. A ghost abstraction: a helper, type, or layer with one caller and no
   likely second caller.
2. A comment or doc comment that restates the symbol's name or the next
   line.
3. Dead weight the section 7 blocker does not already catch: code a test
   asserts on but no run reaches, or commented-out code.
4. Repeated ceremony: the same multi-line pattern at many sites that one
   local helper would remove.
5. Document weight: a markdown file larger than its audience or its purpose
   needs.
6. Test slop: duplicate coverage, a test that asserts the mock, setup that
   restates the implementation.
7. Idiom mismatch: code whose naming, comment density, or shape does not
   match the file around it.

## Repository specifics

- Performance is the oracle. A change that trades performance for
  readability is a blocker unless the design record for it says why. A
  change that trades readability for performance is a blocker when the same
  performance was available with readable code.
- `MotionSample` is the only sensor input to the core. A second path is a
  blocker.
- The reference device is CLAUDE.md section 5. A measurement from any other
  device, or with no date, does not count.
