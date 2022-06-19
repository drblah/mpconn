#!/bin/bash

set -e

(cd ../../ && cargo build --release)
cp ../../target/release/mpconn ./
