Toolchain: you have been assigned the Ikaros runtime for the critical lifecycle. It is provided as local npm
tarballs under `KIT/npm/` (install both into your project, e.g. `npm install --offline --no-audit --no-fund
KIT/npm/ikaros-runtime-0.5.0.tgz KIT/npm/ikaros-darwin-arm64-0.5.0.tgz`; if npm cannot run, extract the tarballs
manually) with its guide at `KIT/README.md`, `KIT/docs/`, and `KIT/examples/`. Use Ikaros for the critical lifecycle
rules and their persistence; use ordinary libraries or built-ins for the rest of the application. Do not modify the
installed runtime. The machine definitions and your application code are yours to write.
