#!/system/bin/sh

MODDIR="${0%/*}"
STATE_ROOT="/data/adb/usb_samplerate_changer_webui"
SETTINGS="$STATE_ROOT/settings.conf"

(
    i=0
    while [ "$i" -lt 60 ] && [ "$(getprop sys.boot_completed)" != "1" ]; do
        sleep 1
        i=$((i + 1))
    done
    [ "$(getprop sys.boot_completed)" = "1" ] || exit 0

    i=0
    while [ "$i" -lt 30 ] && [ "$(getprop init.svc.audioserver)" != "running" ]; do
        sleep 1
        i=$((i + 1))
    done
    [ "$(getprop init.svc.audioserver)" = "running" ] || exit 0
    [ -x "$MODDIR/usbsrctl" ] || exit 0

    # AIDL policy topology is reconstructed by the HAL process. Restore a
    # previously validated module XML before optional tool/tuning reapply.
    "$MODDIR/usbsrctl" aidl-restore >>"$STATE_ROOT/boot-aidl-restore.log" 2>&1 || true

    [ -r "$SETTINGS" ] || exit 0
    grep -q '^auto_reapply=1$' "$SETTINGS" || exit 0
    "$MODDIR/usbsrctl" reapply >>"$STATE_ROOT/boot-reapply.log" 2>&1 || true
) &
