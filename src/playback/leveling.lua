-- Sets each file's volume-gain before it opens, from user-data/shmc/gain/<part id>.

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
    local done = false
    local changed
    local function finish(gain)
        if done then
            return
        end
        done = true
        mp.unobserve_property(changed)
        if gain then
            apply(gain)
        end
        hook:cont()
    end
    changed = function(_, gain)
        if gain then
            finish(gain)
        end
    end
    mp.observe_property(name, "number", changed)
    mp.add_timeout(WAIT_SECONDS, function()
        finish(nil)
    end)
end)
