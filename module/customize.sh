#!/system/bin/sh

ui_print "***************************************"
MODULE_VERSION="$(sed -n 's/^version=//p' "${MODPATH:-.}/module.prop" 2>/dev/null || true)"
ui_print "  USB SampleRate Changer WebUI v${MODULE_VERSION:-unknown}"
ui_print "***************************************"
ui_print "- SolidJS WebUI + Rust controller"
ui_print "- Validated AIDL policy restore at boot"
ui_print "- KernelSU/APatch WebUI required"

# Detect the active backend without changing the audio policy. Traditional
# devices keep the upstream path; AIDL devices use the additive controller.
ui_print "- Checking audio policy backend"
CONFIG_SOURCE="$(dumpsys media.audio_policy 2>/dev/null | awk '
    /^ Config source: / {
        sub(/^ Config source: /, "")
        print
        exit
    }')"

chmod 0755 "$MODPATH/usbsrctl"
if [ "$CONFIG_SOURCE" = "AIDL HAL" ]; then
    ui_print "- AIDL Audio HAL detected"
    if ! "$MODPATH/usbsrctl" status 2>/dev/null | grep -q '^aidl_preview_supported=1$'; then
        abort "! No uniquely matching AIDL primary configuration was found"
    fi
    ui_print "- AIDL module XML adapter verified"
elif [ -n "$CONFIG_SOURCE" ] && [ -r "$CONFIG_SOURCE" ]; then
    ui_print "- Traditional audio policy XML: $CONFIG_SOURCE"
else
    ui_print "! Traditional audio policy XML was not detected"
    if [ -n "$CONFIG_SOURCE" ]; then
        ui_print "! Reported config source is not readable: $CONFIG_SOURCE"
    fi
    abort "! Unsupported audio policy backend"
fi

touch "$MODPATH/skip_mount"
set_perm "$MODPATH/usbsrctl" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/core/USB_SampleRate_Changer.sh" 0 0 0755
set_perm "$MODPATH/core/functions3.shlib" 0 0 0644
set_perm_recursive "$MODPATH/core/templates" 0 0 0755 0644
set_perm_recursive "$MODPATH/core/extras" 0 0 0755 0644
set_perm_recursive "$MODPATH/webroot" 0 0 0755 0644
