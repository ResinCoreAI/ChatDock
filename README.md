# ChatDock site

The download site for ChatDock: static HTML, CSS and ES modules, no build step, no dependencies.

- `index.html`: the home page (the live demo screen, features, apps, latest release, questions)
- `whats-new.html`: every release, read from GitHub Releases
- `assets/i18n.js`: every word, in Thai, English, Chinese, Japanese and German
- `assets/screen.js`: the demo screen (canvas)
- `assets/lib.js`: pure helpers, covered by `npm test`

## Run it

```
npm test
npm run serve        # http://127.0.0.1:5181
```

ES modules need http://, so open it through the server, not as a file.

## Publish

GitHub Pages serves the `gh-pages` branch, which holds the contents of this folder. Publish only when the founder says so.
