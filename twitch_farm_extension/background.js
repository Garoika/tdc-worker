const SERVER_URL = "http://127.0.0.1:5000";
let isServerOnline = false;
let currentProxyAuth = null;

console.log("%c[Twitch Farm Background] 🚀 Service worker initialized", "color: #9146FF; font-weight: bold;");

// Set up Chrome Proxy for Twitch domains only
function applyProxySettings(proxy) {
    if (!proxy || !proxy.host || !proxy.port) {
        clearProxySettings();
        return;
    }

    const host = proxy.host;
    const port = proxy.port;
    currentProxyAuth = (proxy.username && proxy.password) ? {
        username: proxy.username,
        password: proxy.password
    } : null;

    // PAC script routes ONLY *.twitch.tv and twitch.tv traffic through HTTP proxy
    const pacScript = `
        function FindProxyForURL(url, host) {
            if (shExpMatch(host, "*.twitch.tv") || host === "twitch.tv") {
                return "PROXY ${host}:${port}";
            }
            return "DIRECT";
        }
    `;

    chrome.proxy.settings.set(
        {
            value: {
                mode: "pac_script",
                pacScript: { data: pacScript }
            },
            scope: "regular"
        },
        () => {
            console.log(`%c[Background] 🛡️ Proxy enabled for Twitch: ${host}:${port} (Auth: ${currentProxyAuth ? 'Yes' : 'No'})`, "color: #00d2d3; font-weight: bold;");
        }
    );
}

function clearProxySettings() {
    currentProxyAuth = null;
    chrome.proxy.settings.clear({ scope: "regular" }, () => {
        console.log("%c[Background] 🛡️ Proxy cleared, direct connection restored", "color: #ff9f43; font-weight: bold;");
    });
}

// Automatically authenticate proxy requests without browser prompt
chrome.webRequest.onAuthRequired.addListener(
    (details, callback) => {
        if (currentProxyAuth && currentProxyAuth.username && currentProxyAuth.password) {
            return {
                authCredentials: {
                    username: currentProxyAuth.username,
                    password: currentProxyAuth.password
                }
            };
        }
        return {};
    },
    { urls: ["*://*.twitch.tv/*", "*://twitch.tv/*"] },
    ["blocking"]
);

// Completely wipe ALL Twitch cookies across all subdomains (including httpOnly cookies like persistent, sudo, etc.)
async function wipeAllHttpOnlyCookies() {
    return new Promise((resolve) => {
        chrome.cookies.getAll({ domain: "twitch.tv" }, (cookies) => {
            if (!cookies || cookies.length === 0) {
                resolve();
                return;
            }
            let pending = cookies.length;
            cookies.forEach((cookie) => {
                let domain = cookie.domain.startsWith(".") ? cookie.domain.substring(1) : cookie.domain;
                let protocol = cookie.secure ? "https://" : "http://";
                let url = protocol + domain + cookie.path;
                
                chrome.cookies.remove({ url: url, name: cookie.name }, () => {
                    if (chrome.runtime.lastError) {}
                    pending--;
                    if (pending <= 0) resolve();
                });
            });
        });
    });
}

// Inject clean auth-token via native Chrome Cookies API
async function injectCleanAuthToken(authToken) {
    await wipeAllHttpOnlyCookies();

    const targets = [
        { url: "https://www.twitch.tv/", domain: ".twitch.tv" },
        { url: "https://auth.twitch.tv/", domain: "auth.twitch.tv" },
        { url: "https://id.twitch.tv/", domain: "id.twitch.tv" }
    ];

    for (let t of targets) {
        await new Promise((res) => {
            chrome.cookies.set({
                url: t.url,
                domain: t.domain,
                name: "auth-token",
                value: authToken,
                path: "/",
                secure: true,
                expirationDate: Math.floor(Date.now() / 1000) + 31536000
            }, (cookie) => {
                if (chrome.runtime.lastError) {}
                res();
            });
        });
    }
}



async function wipeTwitchSessionCompletely() {
    console.log("%c[Background] 🧹 Wiping Twitch cookies, localStorage and session completely...", "color: #ff4757; font-weight: bold;");
    await wipeAllHttpOnlyCookies();
    
    // Clear storage and remove helper panels in all open Twitch tabs
    chrome.tabs.query({ url: "https://*.twitch.tv/*" }, (tabs) => {
        if (!tabs || tabs.length === 0) return;
        tabs.forEach((tab) => {
            chrome.tabs.sendMessage(tab.id, { action: "WIPE_LOCAL_STORAGE" }, () => {
                if (chrome.runtime.lastError) {}
            });
        });
    });
}

let lastUserCode = null;
let lastProxyKey = null;

async function checkServerStatus() {
    try {
        const res = await fetch(`${SERVER_URL}/api/current`, { cache: "no-store" });
        if (!res.ok) {
            if (isServerOnline) {
                console.log("[Background] ⚠️ Server status returned HTTP " + res.status);
                await wipeTwitchSessionCompletely();
            }
            isServerOnline = false;
            lastUserCode = null;
            return;
        }
        const data = await res.json();
        
        const proxyKey = data.proxy ? `${data.proxy.host}:${data.proxy.port}` : "none";
        
        if (data && data.user_code && (data.user_code !== lastUserCode || proxyKey !== lastProxyKey)) {
            const isProxySwap = (data.user_code === lastUserCode && proxyKey !== lastProxyKey);
            lastUserCode = data.user_code;
            lastProxyKey = proxyKey;
            isServerOnline = true;
            
            console.log(`[Background] 🚀 ${isProxySwap ? 'Proxy swapped for active auth' : 'New auth code detected'}: ${data.user_code} (Proxy: ${proxyKey})`);
            
            // Dynamically apply proxy if provided by server
            if (data.proxy) {
                applyProxySettings(data.proxy);
            } else {
                clearProxySettings();
            }

            openOrFocusTwitchActivate(data.user_code);
        } else if (data && (data.status === "finished" || data.status === "waiting")) {
            if (isServerOnline) {
                console.log("[Background] 🏁 Auth queue finished! Wiping Twitch session completely...");
                await wipeTwitchSessionCompletely();
                clearProxySettings();
            }
            isServerOnline = false;
            lastUserCode = null;
        }
    } catch (e) {
        if (isServerOnline) {
            console.log("[Background] 🔌 Server disconnected, wiping session...");
            await wipeTwitchSessionCompletely();
            clearProxySettings();
        }
        isServerOnline = false;
        lastUserCode = null;
    }
}

function openOrFocusTwitchActivate(userCode) {
    const url = userCode ? `https://www.twitch.tv/activate?device-code=${userCode}` : "https://www.twitch.tv/activate";
    chrome.tabs.query({ url: "https://*.twitch.tv/*" }, (tabs) => {
        if (tabs && tabs.length > 0) {
            chrome.tabs.update(tabs[0].id, { url: url, active: true });
        } else {
            chrome.tabs.create({ url: url });
        }
    });
}

// Set up Chrome Alarms for Manifest V3 background service worker keep-alive
chrome.alarms.create("monitor_server", { periodInMinutes: 0.05 });
chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === "monitor_server") {
        checkServerStatus();
    }
});

// Also check immediately when background service worker wakes up
checkServerStatus();
setInterval(checkServerStatus, 2000);

// Listen for messages from content.js
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.action === "WIPE_AND_INJECT") {
        injectCleanAuthToken(request.authToken).then(() => {
            sendResponse({ success: true });
        });
        return true;
    }
    
    if (request.action === "WIPE_ONLY") {
        wipeAllHttpOnlyCookies().then(() => {
            sendResponse({ success: true });
        });
        return true;
    }
    
    if (request.action === "FETCH_API") {
        const fetchOptions = {
            method: request.method || "GET",
            cache: "no-store",
            headers: {}
        };
        if (request.body) {
            fetchOptions.headers["Content-Type"] = "application/json";
            fetchOptions.body = JSON.stringify(request.body);
        }
        fetch(`${SERVER_URL}${request.endpoint}`, fetchOptions)
            .then(res => {
                if (!res.ok) throw new Error("Not OK: " + res.status);
                return res.json();
            })
            .then(data => sendResponse({ success: true, data: data }))
            .catch(err => sendResponse({ success: false, error: err.toString() }));
        return true;
    }
});
