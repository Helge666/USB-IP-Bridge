#!/bin/bash
# Starts MorningDAW with the XR18 from the Windows PC and gives the XR18 back afterwards.
#
# `usbipb run` brings the XR18 here, waits until it is usable (the device profile says:
# ALSA card, MIDI port and PipeWire node; at most --wait seconds, MorningDAW starts
# anyway after that and the XR18 joins once it is free), runs MorningDAW through
# PipeWire's JACK library and, when it exits, gives the XR18 back – unless it was
# already here before.
#
# Never restarts PipeWire/WirePlumber (a game stream or other audio may be running).
# Example: install as ~/.local/bin/morningdaw-xr18 and point a menu entry at it.

exec usbipb run --device 1397:00d4 --wait "${WAIT_SECONDS:-60}" -- pw-jack "$HOME/.local/opt/MorningDAW/morningdaw" "$@"
