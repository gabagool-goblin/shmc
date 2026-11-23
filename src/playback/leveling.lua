-- Sets each file's volume-gain from user-data/shmc/gain/<part id>: before it opens, or
-- while it plays when the gain arrives after WAIT_SECONDS, undone when the file ends.

if mp.get_property("volume-gain") == nil then
    return
end
mp.set_property_native("user-data/shmc/leveling", true)

local WAIT_SECONDS = 1

mp.add_hook("on_load", 50, function(hook)
    local url = mp.get_property("stream-open-filename", "")
    local part = url:match("/library/parts/(%d+)/")
    if not part then
        return
    end
    local name = "user-data/shmc/gain/" .. part
    local function apply(gain)
        mp.set_property_number("file-local-options/volume-gain", gain)
    end
    local known = mp.get_property_number(name)
    if known then
        apply(known)
        return
    end

    hook:defer()
    local waiting = true
    local changed, ended
    local restore
    local function stop()
        mp.unobserve_property(changed)
        mp.unregister_event(ended)
    end
    local function release()
        if waiting then
            waiting = false
            hook:cont()
        end
    end
    changed = function(_, gain)
        if not gain then
            return
        end
        if waiting then
            apply(gain)
            release()
            stop()
        else
            restore = mp.get_property_number("volume-gain")
            mp.set_property_number("volume-gain", gain)
            mp.unobserve_property(changed)
        end
    end
    ended = function()
        waiting = false
        stop()
        if restore then
            mp.set_property_number("volume-gain", restore)
        end
    end
    mp.observe_property(name, "number", changed)
    mp.register_event("end-file", ended)
    mp.add_timeout(WAIT_SECONDS, release)
end)
