#!/usr/bin/env node
import { App } from './app';

if (!process.stdin.isTTY) {
  console.error('sdeck needs an interactive terminal.');
  process.exit(1);
}

void new App().run();
