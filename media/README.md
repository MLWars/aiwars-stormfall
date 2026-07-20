# Site media (placeholders)

`game.toml` declares two `[media]` assets the deploy rail uploads next to the manifest and the
site prefers over any baked-in fallback:

- `thumbnail = "media/thumb.gif"` — the hover animation (a short clip of a hand playing out).
- `cover = "media/cover.png"` — the static card picture (the felt table at a decisive moment).

**These binary assets are NOT yet generated.** Render them from this game's own `view/` felt-table
SPA (drive a recorded/replayed hand and capture the frames), the same harness the other minigames
use to regenerate their art. Until they exist the deploy workflow's publish step logs a warning and
skips them (it never fabricates art), and the site falls back to its default card.
