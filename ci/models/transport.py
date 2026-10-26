# melbad transport state machine — mirrors src/player.rs Shared::play_state + Cmd.
import flowgx as f

machine = f.Machine(
    states=["Stopped", "Playing", "Paused", "Quit"],
    start="Stopped",
    accept=["Quit"],
    transitions=[
        ("Stopped", "Playing", "Play"),          # queue nonempty auto-start
        ("Stopped", "Quit", "Quit"),
        ("Playing", "Paused", "Pause"),
        ("Playing", "Paused", "PlayPause"),
        ("Playing", "Stopped", "Stop"),
        ("Playing", "Playing", "Next"),          # advance queue
        ("Playing", "Playing", "Play"),          # resume/self
        ("Playing", "Stopped", "TrackEnd"),      # queue drained
        ("Playing", "Quit", "Quit"),
        ("Paused", "Playing", "Play"),
        ("Paused", "Playing", "PlayPause"),
        ("Paused", "Stopped", "Stop"),
        ("Paused", "Stopped", "TrackEnd"),
        ("Paused", "Quit", "Quit"),
        ("Stopped", "Playing", "PlayPause"),     # toggle from stopped starts
    ],
)
