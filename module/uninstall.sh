#!/system/bin/sh

MODDIR="${0%/*}"
STATE_ROOT="/data/adb/usb_samplerate_changer_webui"
LOG_ROOT="/data/local/tmp/usb_samplerate_changer_webui"
reset_failed=0

if [ -x "$MODDIR/usbsrctl" ]; then
    "$MODDIR/usbsrctl" settings audioserver-priority disable >/dev/null 2>&1 || reset_failed=1
    "$MODDIR/usbsrctl" _audioserver-priority-stop >/dev/null 2>&1 || true
fi

if [ -x "$MODDIR/usbsrctl" ]; then
    # Cleanup ignores controller state health, runs every fixed reset and
    # restarts audioserver once before the module files disappear.
    # Bluetooth HAL properties are restored from their ROM-derived baseline.
    "$MODDIR/usbsrctl" cleanup >/dev/null 2>&1 || reset_failed=1
else
    reset_failed=1
fi

if [ "$reset_failed" -eq 0 ]; then
    rm -rf "/data/adb/usb_samplerate_changer_webui"
    rm -rf "/data/local/tmp/usb_samplerate_changer_webui"
else
    echo "USB SampleRate Changer: some audio settings could not be reset during uninstall" 1>&2
    echo "Saved configuration and logs were retained for recovery" 1>&2
fi
