#!/bin/bash

set -e

(cd ../../ && RUSTFLAGS=-g cargo build --release)
cp ../../target/release/mpconn ./
