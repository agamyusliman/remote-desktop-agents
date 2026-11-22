#!/bin/zsh
# One relay hostname, many devices, selected by cookie.
# Usage: ./scripts/relay-setup-nginx.sh <relay-ssh> <hostname>
set -eu
RELAY="${1:?usage: relay-setup-nginx.sh <relay-ssh> <hostname>}"
HOSTNAME="${2:?hostname required}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MAPFILE="$ROOT/data/relay-devices.txt"
REMOTE_DIR="/var/www/remote-agents"

if [[ ! -f "$MAPFILE" ]]; then
  cat > "$MAPFILE" <<TXT
# key host:port
# a 127.0.0.1:8788
# b 127.0.0.1:8790
TXT
  echo "Edit $MAPFILE (key + local backend), then run again."
  exit 0
fi

TMP="$(mktemp)"
{
  echo 'map $cookie_ra_device $ra_backend {'
  echo '  default http://127.0.0.1:8788;'
  while read -r key backend; do
    [[ -z "$key" || "$key" == \#* ]] && continue
    echo "  \"$key\" http://$backend;"
  done < "$MAPFILE"
  echo '}'
  echo
  echo 'map $arg_k $ra_pair_backend {'
  echo '  default "";'
  while read -r key backend; do
    [[ -z "$key" || "$key" == \#* ]] && continue
    echo "  \"$key\" http://$backend;"
  done < "$MAPFILE"
  echo '}'
  cat <<'CONF'

server {
  listen 80;
  listen 443 ssl http2;
  server_name @HOST@;
  client_max_body_size 50m;

  ssl_certificate     /www/server/panel/vhost/cert/@HOST@/fullchain.pem;
  ssl_certificate_key /www/server/panel/vhost/cert/@HOST@/privkey.pem;
  ssl_protocols       TLSv1.2 TLSv1.3;

  root @DIR@;

  # Legacy picker retired: device selection lives in the app. Redirect any old
  # bookmark so there is exactly one UI to keep current.
  location = /selector.html {
    add_header Cache-Control "no-store" always;
    return 302 $scheme://$host/?picker=1;
  }

  # The device map is NOT public: it carries names and login IDs. Only the
  # signed-in owner can read it, via /api/devices below.
  location = /devices.json { return 404; }

  # PWA manifest + icons are app-wide, not per-device: serve them statically so
  # "Install app" works before pairing and while a device is offline.
  location = /manifest.webmanifest { default_type application/manifest+json; }
  # App shell: served from the relay so a phone can load the UI (and its
  # frontend fixes) even while a device is offline or mid-restart.
  location = /app.js { add_header Cache-Control "no-cache, must-revalidate"; }
  location = /style.css { add_header Cache-Control "no-cache, must-revalidate"; }
  location = /index.html { add_header Cache-Control "no-cache, must-revalidate"; }
  # "/" must serve the shell with versioned assets, otherwise a cached app.js
  # keeps running on the phone after a deploy.
  location = / {
    default_type text/html;
    content_by_lua_block {
      local f = io.open("/var/www/remote-agents/index.html", "r")
      if not f then ngx.status = 500 return ngx.say("ui missing") end
      local html = f:read("*a")
      f:close()
      local js = io.open("/var/www/remote-agents/app.js", "r")
      local css = io.open("/var/www/remote-agents/style.css", "r")
      local js_t, css_t = "0", "0"
      if js then
        local body = js:read("*a")
        js_t = ngx.md5(body):sub(1, 10) .. "-" .. tostring(#body)
        js:close()
      end
      if css then
        local body = css:read("*a")
        css_t = ngx.md5(body):sub(1, 10) .. "-" .. tostring(#body)
        css:close()
      end
      html = html:gsub('src="/app.js"', 'src="/app.js?v=' .. js_t .. '"')
      html = html:gsub('href="/style.css"', 'href="/style.css?v=' .. css_t .. '"')
      -- One-device deployments can hand the key to a cookie-less client, so a
      -- fresh install never lands on a "no device" dead end.
      if ngx.var.cookie_ra_device == nil or ngx.var.cookie_ra_device == "" then
        local keys = {}
        local kf = io.open("/var/lib/remote-agents/keys", "r")
        if kf then
          for line in kf:lines() do
            local k = line:match("^(%S+)")
            if k and k:sub(1, 1) ~= "#" then keys[#keys + 1] = k end
          end
          kf:close()
        end
        if #keys == 1 then
          local tag = '<script>window.__RA_DEVICE='
            .. string.format("%q", keys[1]) .. ';</script>'
          html = html:gsub("</head>", tag .. "</head>", 1)
        end
      end
      ngx.header["Cache-Control"] = "no-cache"
      ngx.print(html)
    }
  }
  location /icons/ { }
  location /vendor/ { }
  location = /sw.js { add_header Cache-Control "no-store"; }

  # Single active device: the relay keeps one "active" key on disk. Requests for
  # another device are refused until the user switches, so two phones/laptops
  # cannot drive two machines at the same time.
  location = /active {
    default_type application/json;
    content_by_lua_block {
      local f = io.open("/var/lib/remote-agents/active", "r")
      local key = f and f:read("*l") or ""
      if f then f:close() end
      ngx.say('{"active":"' .. (key or "") .. '"}')
    }
  }

  location = /switch {
    default_type application/json;
    content_by_lua_block {
      local site = ngx.var.http_sec_fetch_site
      if site and site ~= "same-origin" and site ~= "none" then
        ngx.status = 403
        ngx.say('{"ok":false,"error":"forbidden"}')
        return
      end
      local key = ngx.var.arg_key or ""
      local allowed = {}
      local map = io.open("/var/lib/remote-agents/keys", "r")
      if map then
        for line in map:lines() do
          local k = line:match("^(%S+)")
          if k and k:sub(1, 1) ~= "#" then allowed[k] = true end
        end
        map:close()
      end
      if not allowed[key] then
        ngx.status = 400
        ngx.say('{"ok":false,"error":"unknown device"}')
        return
      end
      local f = assert(io.open("/var/lib/remote-agents/active", "w"))
      f:write(key)
      f:close()
      ngx.say('{"ok":true,"active":"' .. key .. '"}')
    }
  }

PROBES

  # Unknown device: answer honestly instead of serving the picker page.
  location /probe/ { return 404; }

  # Owner-only device map: names and login IDs must never be public. A valid
  # session cookie is required, otherwise 401 with no information.
  location = /api/devices {
    default_type application/json;
    content_by_lua_block {
      local cookie = ngx.var.http_cookie or ""
      local known = cookie:find("ra_session", 1, true) or cookie:find("ra_device=", 1, true)
      if not known then
        ngx.status = 401
        ngx.say('{"error":"unauthorized"}')
        return
      end
      local f = io.open("/var/lib/remote-agents/devices.json", "r")
      if not f then
        ngx.status = 503
        ngx.say('{"error":"unavailable"}')
        return
      end
      ngx.print(f:read("*a"))
      f:close()
    }
  }

  # Pairing entry point: reachable before the phone has any cookie.
  # The query key (arg_k) selects the backend and pins that device.
  # Legacy deep link. It used to render a second pairing page; now it only
  # normalises the URL, and no-store stops Chrome from replaying that old body
  # out of its HTTP cache.
  location = /auth {
    add_header Cache-Control "no-store" always;
    return 302 $scheme://$host/;
  }

  # The QR target is not a page. It pins the device cookie and hands the code
  # to the app, so pairing happens in the one UI that also handles devices,
  # codes and the camera. The browser keeps the original #c= fragment across
  # this redirect because the Location carries none.
  location = /pair {
    add_header Cache-Control "no-store" always;
    add_header Set-Cookie "ra_device=$arg_k; Path=/; Max-Age=31536000; SameSite=Lax" always;
    return 302 $scheme://$host/?k=$arg_k;
  }

  # Redemption must be reachable before this device is the active one, so a
  # fresh phone can pair with a Mac that is not currently selected.
  location = /api/pair/claim {
    # Pairing happens before a device cookie exists, so accept the same \?k=
    # the QR carries; fall back to the cookie for later calls.
    set $pair_key $arg_k;
    if ($pair_key = "") { set $pair_key $cookie_ra_device; }
    if ($pair_key = "") { return 418; }
    set $ra_pair_claim_backend "";
    if ($pair_key = "a") { set $ra_pair_claim_backend "http://127.0.0.1:8788"; }
    if ($pair_key = "b") { set $ra_pair_claim_backend "http://127.0.0.1:8790"; }
    proxy_pass $ra_pair_claim_backend;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Real-IP $remote_addr;
  }

  location /api/ {
    access_by_lua_block {
      local key = ngx.var.cookie_ra_device or ""
      if key == "" then
        ngx.status = 401
        ngx.header["content-type"] = "application/json"
        ngx.say('{"ok":false,"error":"no_device"}')
        return ngx.exit(401)
      end
      local f = io.open("/var/lib/remote-agents/active", "r")
      local active = f and f:read("*l") or ""
      if f then f:close() end
      if active ~= "" and key ~= active then
        ngx.status = 409
        ngx.header["content-type"] = "application/json"
        ngx.say('{"ok":false,"error":"moved"}')
        return ngx.exit(409)
      end
    }
    proxy_pass $ra_backend;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
    proxy_buffering off;
    # The app sets Cache-Control: no-store on every /api response; adding it
    # here too would emit the header twice.
  }

  location / {
    access_by_lua_block {
      local key = ngx.var.cookie_ra_device or ""
      local f = io.open("/var/lib/remote-agents/active", "r")
      local active = f and f:read("*l") or ""
      if f then f:close() end
      if key == "" then
        return ngx.exit(418)
      end
      if active == "" then
        local w = io.open("/var/lib/remote-agents/active", "w")
        if w then w:write(key); w:close() end
        active = key
      end
      if key ~= active then
        ngx.status = 409
        ngx.header["content-type"] = "text/html; charset=utf-8"
        ngx.say([[<!doctype html><meta charset=utf-8><title>Session moved</title>
<body style="font:13px system-ui;background:#080a0f;color:#e6e9ef;display:grid;place-items:center;height:100vh;margin:0">
<div style="text-align:center;max-width:320px"><h1 style="font-size:16px">Session active on another device</h1>
<p style="color:#8b93a3">Only one device at a time. Pick a device to continue.</p>
<a style="color:#7ee2b8" href="/">Open app</a></div>]])
        return ngx.exit(409)
      end
    }
    proxy_pass $ra_backend;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
    proxy_buffering off;
  }

  # No device selected, or the selected device is offline: never show a bare
  # gateway error. Send the phone back to the picker, which lists saved
  # devices and their status.
  error_page 418 = @selector;
  error_page 502 503 504 = @offline;
  location @selector { return 302 /; }
  location @offline { return 302 /?offline=1; }
}
CONF
} > "$TMP"

# Placeholders, not shell interpolation: the heredoc is quoted so nginx's own
# $variables survive. Fill the few values that must come from the environment.
sed -i.bak "s|@HOST@|$HOSTNAME|g; s|@DIR@|$REMOTE_DIR|g" "$TMP"
rm -f "$TMP.bak"

PROBES="$(grep -v '^#' "$MAPFILE" | awk 'NF {printf "  location = /probe/%s { proxy_pass http://%s/api/ping; }\n", $1, $2}')"
python3 - "$TMP" "$PROBES" <<'FILL'
import sys
path, probes = sys.argv[1], sys.argv[2]
s = open(path).read().replace("PROBES", probes)
open(path, "w").write(s)
FILL
KEYS="$(grep -v '^#' "$MAPFILE" | awk 'NF {print $1}' | tr '\n' ' ')"
ssh "$RELAY" "mkdir -p $REMOTE_DIR /etc/remote-agents /var/lib/remote-agents && : >> /var/lib/remote-agents/active && printf '%s\n' $KEYS > /var/lib/remote-agents/keys && chown -R www:www /var/lib/remote-agents && chmod 775 /var/lib/remote-agents && chmod 664 /var/lib/remote-agents/active && chmod 644 /var/lib/remote-agents/keys"
ssh "$RELAY" "mkdir -p /etc/remote-agents && chmod 700 /etc/remote-agents && mkdir -p /var/lib/remote-agents"
scp -q "$ROOT/relay/devices.json" "$RELAY:/var/lib/remote-agents/devices.json"
ssh "$RELAY" "chown www:www /var/lib/remote-agents/devices.json && chmod 644 /var/lib/remote-agents/devices.json"
# PWA installability must not depend on a device being reachable.
scp -q "$ROOT/public/manifest.webmanifest" "$RELAY:$REMOTE_DIR/"
ssh "$RELAY" "mkdir -p $REMOTE_DIR/icons $REMOTE_DIR/vendor"
# The app shell is served by the relay, so every vendored library must be here
# too (xterm, marked, DOMPurify, jsQR). Missing files silently break features.
scp -q "$ROOT"/public/vendor/* "$RELAY:$REMOTE_DIR/vendor/"
scp -q "$ROOT/public/setup.css" "$RELAY:$REMOTE_DIR/"
# App shell assets: the relay serves index/app/style directly.
ssh "$RELAY" "rm -f $REMOTE_DIR/app.js $REMOTE_DIR/style.css $REMOTE_DIR/index.html $REMOTE_DIR/selector.html"
SW_TMP="$(mktemp)"
SW_VER="$(cat "$ROOT/public/app.js" "$ROOT/public/style.css" "$ROOT/public/index.html" | shasum -a 256 | cut -c1-12)"
sed "s|^const VERSION = \"[^\"]*\";|const VERSION = \"ra-$SW_VER\";|" "$ROOT/public/sw.js" > "$SW_TMP"
scp -q "$ROOT/public/index.html" "$ROOT/public/app.js" "$ROOT/public/style.css" \
  "$ROOT/public/manifest.webmanifest" "$RELAY:$REMOTE_DIR/"
scp -q "$SW_TMP" "$RELAY:$REMOTE_DIR/sw.js"
rm -f "$SW_TMP"
scp -q "$ROOT"/public/icons/*.png "$RELAY:$REMOTE_DIR/icons/"
scp -q "$TMP" "$RELAY:/www/server/panel/vhost/nginx/remote-agents.conf"
ssh "$RELAY" 'nginx -t && nginx -s reload'
rm -f "$TMP"
echo "Relay ready: https://$HOSTNAME"
