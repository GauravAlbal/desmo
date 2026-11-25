#!/usr/bin/env node
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const target=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../controls/reference/run.mjs');
const child=spawnSync(process.execPath,[target,...process.argv.slice(2)],{stdio:'inherit',env:process.env});
if(child.error){process.stderr.write(String(child.error)+'\n');process.exit(1)}process.exit(child.status??1);
