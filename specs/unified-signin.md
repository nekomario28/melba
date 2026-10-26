# Unified GUI and managed-wrapper sign-in

Status: **candidate implementation complete and locally verified against fake
native backends; not installed, not committed, live acceptance not started**.

Implemented (uncommitted) in this tree: `melbad auth gui [--restore]`, a small
presentation bridge (`packaging/wrapper-auth.c`) loaded into the wrapper
process, a typed bounded protocol (`crates/melba-auth/`), attempt correlation
in `melbad` and the GUI, a private session lock, tombstone bindings, and one
host-side lifetime owner for every locally spawned native process group
(`packaging/process-supervisor.c`, `src/wrapper/process.rs`). Apple's SDK still
owns the account protocol; nothing here reimplements it.

## Next pickup

Gate 1 is still open: prove against the **real** payload that a user-selected,
Apple-offered delivery action (device notification, text message or call to a
registered trusted number) reaches the same native session. The bridge forwards
whatever prompts and buttons the SDK actually supplies — it does not invent
phone controls — but no live SDK run has confirmed that such choices are offered
or that a delivery request is accepted. Only separately authorized live
acceptance with a disposable account can close this.

## Approved scope

The maintainer requested that GUI login also authenticate the wrapper, instead
of requiring a separate CLI login. They explicitly approved transient GUI
handling of Apple ID, password and verification codes. They also require the
alternative to Apple-device notification: requesting a verification code using
a trusted phone number, as offered by the official client.

This is a narrow exception to the original no-password-collection boundary.
It does not authorize remote credential brokers, account recovery automation,
DRM changes, a new playback backend, or changes to the user's running installation.
Prefer an existing, suitable OSS authentication interface to reimplementing
Apple's account protocol. Keep the native Rust GUI and `melbad` ownership.

## User-facing contract

- One GUI-initiated authentication flow covers the local managed wrapper and
  Melba's MusicKit access. Do not silently combine different signed-in accounts.
- Initial device notification is acceptable. The code step must also provide a
  route equivalent to "Didn't get a code?" rather than only a six-digit field.
- Offer selection of an Apple-registered trusted phone number, displaying only
  the masked number, and SMS or voice call **when Apple actually offers it**.
  Do not accept arbitrary destination numbers or invent delivery options.
- Changing delivery method and requesting a resend are distinct operations from
  submitting an already received code. Do not label changing a local radio
  button as sending an SMS. Show awaiting-delivery only after upstream accepts
  the request. Preserve the challenge on a recoverable delivery error.
- Preserve the user's chosen delivery method during retry. A stale response must
  not revert a newer choice, complete a cancelled login, or attach to another
  attempt. Correlate every response with its attempt and challenge.
- Make cancellation available during login and verification. Terminate and reap
  only owned work; do not kill an unrelated wrapper listening on the same port.
- Respect Apple-side resend limits. No automatic SMS/call retry loops. An explicit
  user action initiates delivery; tests never contact real phone numbers.
- Do not announce complete sign-in until native wrapper authentication and
  server validation of its session-derived MusicKit token both succeed.
  A saved file, successful process exit, cached token or open TCP port is not
  proof of those two facts.

## Security and architecture invariants

One coordinator owns the attempt, native challenge, delivery selection,
verification result, cancellation and final publication. Views emit actions;
blocking child-process, filesystem and network operations stay off the UI thread.

Credentials are transient and excluded from application logs, Debug output,
settings and saved UI state. Use a private, bounded local control channel, not
password command-line arguments or a configurable remote HTTP destination.
Any unavoidable upstream credential-transport limitation must be resolved or
explicitly surfaced before claiming the new GUI flow is safe. Do not mistake
argv scrubbing for eliminating an argv exposure window.

Publish the verified MusicKit token using the existing private atomic store;
retain the old token if the new flow fails. Existing file formats and locations
remain compatible; no migration or new preference is needed. A retained token
must not itself complete a new login attempt. Wrapper session identity and the
MusicKit token must have the same native authentication authority.

Do not use an independent iCloud login to obtain a phone code and assume it
belongs to a pending Music authentication challenge. Do not disable TLS
verification, suppress MFA, or substitute an application-specific password.

## Evidence and donor decisions

- Apple documents code delivery to trusted devices and trusted phone numbers,
  by text message or phone call:
  <https://support.apple.com/ja-jp/102606>.
  This establishes the official account capability, not wrapper compatibility.
- `WorldObservationLog/wrapper`, main at
  `4d83f2feb396ca26847530ff7010ac6a4877f136`: `main.c`'s
  `dialogHandler` selects "Use Existing Apple ID" only for the initial
  "Sign In" dialog. Other buttons are printed, then the callback sends a
  response without forwarding a user selection. `credentialHandler` accepts
  a device verification code via stdin or `2fa.txt`; it exposes no phone-delivery
  control. Its account endpoint supplies a post-login MusicKit token.
- The same project's lite branch at
  `2fd7e0eaef081f93cf687428b965674937832ba4`, `lite/auth.cpp`, has the same
  dialog-selection gap. Its service protocol differs from the current bundle.
- `glomatico/wrapper-v2` at
  `100e0a864e883e03a3ac450a780dd9563fff5271` (Unlicense) exposes structured
  login and code submission, but the inspected `auth.cpp`/`runtime.cpp` still
  do not expose phone delivery or generic dialog selection. **REFERENCE_ONLY**:
  a candidate interface donor, not evidence of an available SMS implementation.
- `WorldObservationLog/wrapper-manager` v2 at
  `4a8859d066af8d9da99ef25006a449b3f161bbd8` (AGPL-3.0) accepts a code and
  writes it to `2fa.txt`. The inspected `login.go` does not establish phone
  delivery. **REFERENCE_ONLY**; do not import a hosted multi-account manager
  or its authentication storage into the desktop application.
- Static exports in the inspected `libstoreservicescore.so` include
  `CredentialsRequest::dialog`, protocol buttons and selected-button response
  methods. Those names do not establish a working phone-delivery path.

Public sources and experiment receipts are retained outside the repository in
`~/.cache/melba-unified-signin-research/`. No donor code was copied into product
code. The native SDK was inspected statically, not executed. No real account,
browser profile, token, SMS or phone call was used.

## Current implementation and what it is proven to do

- The bridge is **freestanding** (`packaging/wrapper-auth.c`): it imports no
  libc symbol at all (raw syscalls, no stack protector) and resolves the SDK's
  functions itself by walking the loaded images' dynamic tables. This is not a
  style choice — measured on the real payload, an LD_PRELOAD library that
  imports libc fails to load (`linker: cannot locate symbol "write"`), while a
  dependency-free library loads in the chrooted payload (namespace PID 1).
- Against the **real installed payload**, the real launcher was run with an
  isolated copy of the wrapper and its account-scoped state removed, with no
  credentials submitted: the bridge loaded, the coordinator received the
  payload's own sign-in dialog *"Sign In"* with its buttons
  *Use Existing Apple ID* / *Create New Apple ID* / *Cancel*, and the attempt
  was then cancelled. Working injection recipe: `LD_PRELOAD=<basename of the
  staged .so>` with `LD_LIBRARY_PATH=<host path of rootfs/system/lib64>` (the
  launcher mirrors host paths inside its chroot).

- `probe-dialog.py` compiled the exact pinned upstream `dialogHandler` against a
  fake SDK: the initial sign-in choice is a positive control, and a synthetic
  verification dialog with Cancel / Text Me / Call Me showed that the upstream
  callback returned without the requested selection. `dialog-receipt.json` binds
  the source, extracted callback, header, probe and results by digest. This
  justifies this repository's own bridge rather than an upstream patch.
- The production coordinator plus the embedded bridge were exercised through an
  isolated fake SDK/validator/payload check that keeps the real protocol,
  process, locking and publication code in the path. The fake SDK retains its
  response object after the callback, so a stack-allocated or wrongly sized
  response would be caught. Scenarios: device notification, offered text/call
  choices with their exact ordinals, wrong code, cached code, resume under an
  authorized binding, wrong password, SDK-rejected sign-in, rejected delivery,
  stale token, zero exit, validator rejection, cancellation, cancellation
  during commit, and owner/peer loss.
- The **real rootless host launcher** was executed with a private glibc fake SDK
  and a fake rootfs, inside a PID namespace: owned processes were reaped and the
  session lock released when the owner process died.
- Layout, prompt-action and privacy regressions run in the GUI suite (including
  a test that every control stays inside the sign-in card at 1000x720 and
  760x520), and 20 software-GL captures of the prompt states exist outside the
  repository for visual review.
- The process-lifetime guards each have a mutation that makes their regression
  fail (stopped supervisor, escaped descendant, already-reaped supervisor,
  symlinked staging ancestor).

What none of this proves: that Apple's live service offers these choices, that a
requested delivery is sent, that the wrapped payload's ABI matches beyond the
statically identified x86_64 files, or that a real account can complete the
flow. It is not an upstream patch, a live authentication result, a rendered
acceptance of the real prompts, or an installed feature.

## Known limitations

- Process lifetime is owned by a supervisor child that leads the worker's
  process group; the owner signals only the group of a PID it still holds
  unreaped, resumes a stopped supervisor before asking it to stop, and refuses
  to supervise at all when `SIGCHLD` is ignored or `SA_NOCLDWAIT` is set. Three
  residual windows remain, all requiring an actor that interferes rather than a
  normal failure: (a) an embedder that changes `SIGCHLD` after start can race
  the microsecond between the liveness check and the signal — the owner then
  signals nothing and leaks instead; (b) an owner process that dies while its
  supervisor is `SIGSTOP`ed leaves that supervisor (and its descendants)
  unswept, because the supervisor's parent-death poll cannot run while it is
  stopped — nothing else in the system stops our supervisor; (c) a supervisor
  killed before its bounded sweep finishes can leave a descendant that called
  `setsid()` alive. The owner waits longer than the supervisor's one-second
  sweep before escalating, so a slow-but-running sweep is never cut short.
- The bridge ABI is pinned to the exact x86_64 payload hashes in
  `packaging/wrapper-auth-abi.json`; any other payload fails closed with
  `unsupported native authentication ABI`.
- `melbad wrapper login` (legacy CLI) still transports the password as a wrapper
  argument; scrubbing is best effort. Prefer the GUI or `melbad auth gui`.
- Failure text the daemon forwards to the GUI (`AuthStatus::Failed`) is English
  today; the UI does not translate daemon-side strings.
- A failed attempt after enrolment blocks the managed wrapper until a valid
  sign-in completes. `melbad auth clear` removes the saved token *and* the
  binding marker (regression: `clearing_sign_in_forgets_a_failed_binding`), so
  the user is never stranded without a way back.
- Staging refuses a symlinked ancestor only for artifacts this code stages; the
  user's own XDG path components are not otherwise rewritten.
- The curated managed wrapper is still the only supported native backend, and
  the release package predates these changes.

The REA ELF-layout operation was unavailable because its pwntools provider was
not configured. GNU `nm`/`objdump` were used for limited static symbol/code
inspection instead. These are shell observations, not REA evidence or runtime
proof. No analysis-engine installation or unrelated environment change was made.

## Acceptance gates

1. **Native capability — PARTIAL.** The real payload's own challenge now reaches
   this coordinator (see above). Still open: a *credentialed* run against a
   disposable account, and proof that a selected verification method (text
   message / call) is offered and accepted by Apple in the same session.
2. **Coordinator — PARTIAL.** The isolated fake peer covers the successful legs,
   device/text/call choices, delivery failure, rejected code, stale token,
   cancellation, commit failure and owner loss, and the selection-loss and
   stale-token-success defects now fail permanently. Expiry and resend limiting
   are not yet exercised.
3. **Production integration — PARTIAL.** The actual child and IPC entry points
   run, with bounded packets, secret-free failures, private files and owned
   cleanup; the real launcher is exercised only with a fake payload, so the
   production ABI and Bionic path remain unverified.
4. **GUI — PARTIAL.** Prompt, choice, error and privacy states are rendered and
   driven in the suite, with 20 captures outside the repository. Rendered
   acceptance of the real prompts (long Apple strings, accessibility, a live
   native prompt) and the unprimed critique of the captures are still open.
5. **Live acceptance — NOT STARTED.** Requires separate authorization and a
   disposable account: device notification, whatever delivery methods Apple
   offers, same-account native and MusicKit success, and cancellation. A mock SMS
   is never live acceptance.

Do not rebuild the saved release package or alter the live daemon as a side
effect of this spike. The earlier installer/package receipts describe older
source snapshots and do not verify this proposed sign-in flow.
