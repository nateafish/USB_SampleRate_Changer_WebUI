#!/system/bin/sh

ui_print "***************************************"
MODULE_VERSION="$(sed -n 's/^version=//p' "${MODPATH:-.}/module.prop" 2>/dev/null || true)"
ui_print "  USB SampleRate Changer WebUI v${MODULE_VERSION:-unknown}"
ui_print "***************************************"
ui_print "- SolidJS WebUI + Rust controller"
ui_print "- No boot service; usable immediately"
ui_print "- KernelSU/APatch WebUI required"

# Match the upstream compatibility check without creating .config or changing
# the active audio policy. Recent AIDL-only devices do not expose a readable
# traditional audio policy XML through media.audio_policy's "Config source".
ui_print "- Checking traditional audio policy XML support"
POLICY_FILE="$(dumpsys media.audio_policy 2>/dev/null | awk '
    /^ Config source: / {
        print $3
        exit
    }')"

if [ -z "$POLICY_FILE" ] || [ ! -r "$POLICY_FILE" ]; then
    ui_print "! Traditional audio policy XML was not detected"
    if [ -n "$POLICY_FILE" ]; then
        ui_print "! Reported policy file is not readable: $POLICY_FILE"
    fi
    abort "! Recent AIDL-only devices are not supported"
fi

ui_print "- Audio policy XML: $POLICY_FILE"

# One-time install/upgrade migration for the legacy controller layout. Keep
# every removal target as a literal absolute path so runtime variables cannot
# broaden or redirect the deletion scope.
ui_print "- Migrating legacy controller runtime files"
if [ -L "/data/adb/usb_samplerate_changer_webui/generated" ]; then
    rm -f "/data/adb/usb_samplerate_changer_webui/generated"
elif [ -d "/data/adb/usb_samplerate_changer_webui/generated" ]; then
    rm -rf "/data/adb/usb_samplerate_changer_webui/generated"
fi
if [ -f "/data/adb/usb_samplerate_changer_webui/last.log" ] || [ -L "/data/adb/usb_samplerate_changer_webui/last.log" ]; then
    rm -f "/data/adb/usb_samplerate_changer_webui/last.log"
fi
if [ -f "/data/adb/usb_samplerate_changer_webui/last.status" ] || [ -L "/data/adb/usb_samplerate_changer_webui/last.status" ]; then
    rm -f "/data/adb/usb_samplerate_changer_webui/last.status"
fi
if [ -f "/data/adb/usb_samplerate_changer_webui/last-command.log" ] || [ -L "/data/adb/usb_samplerate_changer_webui/last-command.log" ]; then
    rm -f "/data/adb/usb_samplerate_changer_webui/last-command.log"
fi
if [ -f "/data/adb/usb_samplerate_changer_webui/boot-reapply.log" ] || [ -L "/data/adb/usb_samplerate_changer_webui/boot-reapply.log" ]; then
    rm -f "/data/adb/usb_samplerate_changer_webui/boot-reapply.log"
fi

touch "$MODPATH/skip_mount"
set_perm "$MODPATH/usbsrctl" 0 0 0755
set_perm "$MODPATH/core/USB_SampleRate_Changer.sh" 0 0 0755
set_perm "$MODPATH/core/functions3.shlib" 0 0 0644
set_perm_recursive "$MODPATH/core/templates" 0 0 0755 0644
set_perm_recursive "$MODPATH/core/extras" 0 0 0755 0644
set_perm_recursive "$MODPATH/webroot" 0 0 0755 0644
