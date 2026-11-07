# Environment variables

[README](../README.md) · [Security and threat model](security.md) · [Authentication](authentication.md)

Melba reads its configuration from `config.json` (see
[Security](security.md#what-is-stored-and-where)). These variables change
behaviour for one process, which is what a wrapper script or a systemd unit
needs. There is no `.env` file: export them in the session that starts Melba, or
set them in the unit's `Environment=`.

## Supported

| Variable | Effect |
|---|---|
| `MELBA_STORE` | Storefront for catalogue requests (default `jp`). Use the country code of the account the token belongs to. |
| `MELBA_MUSIC_USER_TOKEN_CMD` | Shell command whose stdout is the MusicKit user token, instead of the file. See [Security](security.md#what-is-stored-and-where). |
| `MELBA_WRAPPER_ALLOW_REMOTE=1` | Accept a wrapper URL that is not on the loopback interface. Equivalent to `"wrapper_allow_remote": true` in `config.json`; both leave the token and the wrapper's unauthenticated ports exposed to that network. |
| `MELBA_AUTH_PROFILE` | Browser profile directory to import a sign-in from, when the automatic search picks the wrong one. |
| `MELBA_WRAPPER_PASSWORD` | Non-interactive password for `melbad wrapper login`. It is visible in `/proc/<pid>/environ` to anything running as you, so prefer the GUI flow, which hands the password over through a memfd and never writes it. |

Standard XDG variables are honoured and are the supported way to move state:
`XDG_CONFIG_HOME` (token, `config.json`), `XDG_STATE_HOME` (logs, saved queue),
`XDG_DATA_HOME` (wrapper files), `XDG_CACHE_HOME` (developer token, artwork).

## Internal

`MELBA_LOGIN_APPLIED`, `MELBA_AUTH_FD`, `MELBA_AUTH_REUSE`,
`MELBA_CONTEXT_PRIVATE_BUS`, `MELBA_CONTEXT_PARENT_BUS`, `MELBA_LOGIN_FD` and
the `MELBA_*_CHILD`/`MELBA_*_FIXTURE` variables exist for the credential handoff
between processes and for the test harness. They are not an interface: their
names and meanings change with the code that sets them.
