#!/system/bin/sh

MODDIR="${0%/*}"
STATE_ROOT="/data/adb/usb_samplerate_changer_webui"
LOG_ROOT="/data/local/tmp/usb_samplerate_changer_webui"
reset_failed=0

run_reset() {
    if [ -x "$MODDIR/usbsrctl" ]; then
        "$MODDIR/usbsrctl" "$@" >/dev/null 2>&1 || reset_failed=1
    else
        reset_failed=1
    fi
}

if [ -x "$MODDIR/usbsrctl" ]; then
    # Restore module-managed audio settings before the module files disappear.
    # Bluetooth HAL properties are intentionally left untouched.
    run_reset extra resampler reset
    run_reset extra usb-period reset
    run_reset extra jitter disable all
    run_reset extra jitter disable battery
    run_reset extra jitter disable effect
    run_reset reset
fi

rm -rf "/data/adb/usb_samplerate_changer_webui"
rm -rf "/data/local/tmp/usb_samplerate_changer_webui"

if [ "$reset_failed" -ne 0 ]; then
    echo "USB SampleRate Changer: some audio settings could not be reset during uninstall" 1>&2
fi
