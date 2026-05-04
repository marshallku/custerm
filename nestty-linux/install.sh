#!/bin/bash
set -e

echo "Building nestty..."
cargo build --release -p nestty-linux

echo "Installing binary..."
sudo install -Dm755 target/release/nestty /usr/local/bin/nestty

echo "Installing desktop entry..."
sudo install -Dm644 nestty-linux/nestty.desktop /usr/share/applications/nestty.desktop

echo "Done. nestty is now available as a system terminal."
echo "You can set it as default with: gsettings set org.gnome.desktop.default-applications.terminal exec nestty"
