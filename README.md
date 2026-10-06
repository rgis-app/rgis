# rgis

Geospatial data viewer written in Rust. Available on the web ([rgis.app](https://rgis.app)) or natively on desktop.

<img width="700" alt="Screenshot 2023-01-02 at 12 32 18 PM" src="https://user-images.githubusercontent.com/416575/210263423-16b67cfc-5381-4e0a-8b3e-33f289c3a14b.png">

## License

rgis is released under [The Anti-Capitalist Software License (version 1.4)](https://anticapitalist.software/).

## Install

```sh
cargo install --git https://github.com/frewsxcv/rgis
```

## Usage

Run rgis:

```sh
rgis
```

Print help information:

```sh
rgis --help
```

## Driving rgis programmatically

rgis can run a script of commands, then save a screenshot and a JSON dump of
its state (layers, CRS, extents, camera, open windows, errors) and exit:

```sh
cargo run -p rgis -- \
  --script '[{"cmd": "load_file", "path": "countries.geojson"},
             {"cmd": "change_crs", "epsg": 3857},
             {"cmd": "wait_idle"}]' \
  --dump-state out.json \
  --screenshot out.png
```

The web build exposes the same commands and state to JavaScript as
`dispatch(json)` and `get_app_state()`. See
[docs/automation.md](docs/automation.md) for the command list, the state
format, the Playwright helpers, and a macOS caveat about covered windows.
