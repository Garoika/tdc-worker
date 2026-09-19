const SERVER_URL = "http://127.0.0.1:5000";
let isServerOnline = false;

console.log("%c[Twitch Farm Background] 🚀 Service worker initialized", "color: #9146FF; font-weight: bold;");

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

let lastAuthorizeUrl = null;

async function checkServerStatus() {
    try {
        const res = await fetch(`${SERVER_URL}/api/current`, { cache: "no-store" });
        if (!res.ok) {
            if (isServerOnline) {
                console.log("[Background] ⚠️ Server status returned HTTP " + res.status);
                await wipeTwitchSessionCompletely();
            }
            isServerOnline = false;
            lastAuthorizeUrl = null;
            return;
        }
        const data = await res.json();
        
        if (data && data.authorize_url && data.authorize_url !== lastAuthorizeUrl) {
            lastAuthorizeUrl = data.authorize_url;
            isServerOnline = true;
            console.log(`[Background] 🚀 New OAuth auth detected for: ${data.login || data.index}`);
            openOrFocusTwitch(data);
        } else if (data && (data.status === "finished" || data.status === "waiting")) {
            if (isServerOnline) {
                console.log("[Background] 🏁 Auth queue finished! Wiping Twitch session completely...");
                await wipeTwitchSessionCompletely();
            }
            isServerOnline = false;
            lastAuthorizeUrl = null;
        }
    } catch (e) {
        if (isServerOnline) {
            console.log("[Background] 🔌 Server disconnected, wiping session...");
            await wipeTwitchSessionCompletely();
        }
        isServerOnline = false;
        lastAuthorizeUrl = null;
    }
}

function openOrFocusTwitch(data) {
    // First inject auth cookie, then navigate to OAuth authorize URL
    const doNavigate = () => {
        const url = data.authorize_url;
        chrome.tabs.query({ url: "https://*.twitch.tv/*" }, (tabs) => {
            // Also check id.twitch.tv tabs
            chrome.tabs.query({ url: "https://id.twitch.tv/*" }, (idTabs) => {
                const allTabs = [...(tabs || []), ...(idTabs || [])];
                if (allTabs.length > 0) {
                    chrome.tabs.update(allTabs[0].id, { url: url, active: true });
                } else {
                    chrome.tabs.create({ url: url });
                }
            });
        });
    };

    if (data.auth_token) {
        injectCleanAuthToken(data.auth_token).then(doNavigate);
    } else {
        doNavigate();
    }
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

    if (request.action === "POST_TOKEN") {
        fetch(`${SERVER_URL}/api/token`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ access_token: request.accessToken })
        })
            .then(res => res.json())
            .then(data => sendResponse({ success: true, data: data }))
            .catch(err => sendResponse({ success: false, error: err.toString() }));
        return true;
    }
});
