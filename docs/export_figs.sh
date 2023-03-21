#!/bin/bash
cd "$(dirname "$0")"
set -ex
drawio-exporter --drawio-desktop-headless --scale 10 --transparent -f png
