const SERVER_URL = "http://localhost:5000";
let isServerOnline = false;

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

let currentProxy = null;

// Handle Proxy Auth challenges (user/pass authentication)
chrome.webRequest.onAuthRequired.addListener(
    (details) => {
        if (details.isProxy && currentProxy && currentProxy.username) {
            return {
                authCredentials: {
                    username: currentProxy.username,
                    password: currentProxy.password || ""
                }
            };
        }
    },
    { urls: ["<all_urls>"] },
    ["blocking"]
);

function applyProxyConfig(proxyObj) {
    if (!proxyObj || !proxyObj.host || !proxyObj.port) {
        clearProxyConfig();
        return;
    }

    const hostStr = String(proxyObj.host).trim();
    const portInt = parseInt(proxyObj.port);

    if (currentProxy && currentProxy.host === hostStr && currentProxy.port === portInt) {
        return; // Already active
    }

    currentProxy = {
        host: hostStr,
        port: portInt,
        username: proxyObj.username || "",
        password: proxyObj.password || ""
    };

    const config = {
        mode: "fixed_servers",
        rules: {
            singleProxy: {
                scheme: "http",
                host: hostStr,
                port: portInt
            },
            bypassList: ["localhost", "127.0.0.1", "::1"]
        }
    };

    chrome.proxy.settings.set({ value: config, scope: "regular" }, () => {
        console.log(`[Background] 🌐 HTTP Proxy ENABLED: ${hostStr}:${portInt}`);
    });
}

function clearProxyConfig() {
    if (currentProxy === null) return;
    currentProxy = null;
    chrome.proxy.settings.clear({ scope: "regular" }, () => {
        console.log("[Background] 🛑 HTTP Proxy DISABLED (Direct connection restored)");
    });
}

async function checkServerStatus() {
    try {
        const res = await fetch(`${SERVER_URL}/api/current`, { cache: "no-store" });
        if (!res.ok) {
            isServerOnline = false;
            clearProxyConfig();
            return;
        }
        const data = await res.json();

        if (data && data.proxy) {
            applyProxyConfig(data.proxy);
        } else {
            clearProxyConfig();
        }
        
        if (data && (data.user_code || data.auth_token) && !isServerOnline) {
            isServerOnline = true;
            console.log("[Background] 🚀 Python Server detected ONLINE & ACTIVE! Opening Twitch Activate page...");
            openOrFocusTwitchActivate(data.user_code);
        } else if (data && (data.status === "finished" || data.status === "waiting")) {
            isServerOnline = false;
            clearProxyConfig();
        }
    } catch (e) {
        isServerOnline = false;
        clearProxyConfig();
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
        fetch(`${SERVER_URL}${request.endpoint}`, { cache: "no-store" })
            .then(res => {
                if (!res.ok) throw new Error("Not OK");
                return res.json();
            })
            .then(data => sendResponse({ success: true, data: data }))
            .catch(err => sendResponse({ success: false, error: err.toString() }));
        return true;
    }
});
