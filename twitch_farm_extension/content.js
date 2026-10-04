const SERVER_URL = "http://127.0.0.1:5000";

// Listen for storage wipe commands from background service worker
if (chrome && chrome.runtime && chrome.runtime.onMessage) {
    chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
        if (request && request.action === "WIPE_LOCAL_STORAGE") {
            try {
                localStorage.clear();
                sessionStorage.clear();
                console.log("%c[Content] 🧹 LocalStorage and SessionStorage cleared", "color: #ff4757; font-weight: bold;");
                sendResponse({ success: true });
            } catch (e) {
                sendResponse({ success: false, error: e.message });
            }
        }
    });
}

function apiFetch(endpoint, method = "GET", body = null) {
    return new Promise((resolve, reject) => {
        if (!chrome || !chrome.runtime || !chrome.runtime.sendMessage) {
            return reject(new Error("Extension context invalidated"));
        }
        try {
            chrome.runtime.sendMessage({ action: "FETCH_API", endpoint, method, body }, (response) => {
                if (chrome.runtime.lastError) {
                    return reject(chrome.runtime.lastError);
                }
                if (response && response.success) {
                    resolve(response.data);
                } else {
                    reject(new Error(response ? response.error : "Server offline"));
                }
            });
        } catch (e) {
            reject(e);
        }
    });
}

function getActiveSession() {
    return new Promise(resolve => chrome.storage.local.get(['active_user_code'], res => resolve(res.active_user_code)));
}

function setActiveSession(val) {
    return new Promise(resolve => {
        if (val === null) chrome.storage.local.remove('active_user_code', resolve);
        else chrome.storage.local.set({active_user_code: String(val)}, resolve);
    });
}

let isReportingBan = false;

// Check DOM for Twitch proxy ban banner
function checkAndReportProxyBan() {
    if (isReportingBan) return;

    const alertElements = document.querySelectorAll('div[role="alert"], .tw-in-feature-notification, div.dMeebh');
    for (const el of alertElements) {
        const text = (el.textContent || '').toLowerCase();
        if (
            text.includes("your browser is not currently supported") || 
            text.includes("recommended browser") || 
            text.includes("браузер не поддерживается")
        ) {
            isReportingBan = true;
            console.error("%c[Extension] 🚨 Twitch detected proxy/unsupported browser! Reporting to worker...", "color: #ff3838; font-weight: bold; font-size: 14px;");
            
            // Notify local worker agent on port 5000
            apiFetch('/api/report_proxy_dead', 'POST', {}).catch(err => {
                console.error("[Extension] Failed to report dead proxy:", err);
            });
            break;
        }
    }
}

// Observe DOM mutations to immediately detect the error alert as soon as Twitch injects it
const banObserver = new MutationObserver(() => {
    checkAndReportProxyBan();
});

if (document.body) {
    banObserver.observe(document.body, { childList: true, subtree: true });
} else {
    document.addEventListener("DOMContentLoaded", () => {
        banObserver.observe(document.body, { childList: true, subtree: true });
    });
}

async function handleAccountFlow() {
    console.log("%c[Twitch Farm Helper] 🚀 Background agent active on page (UI overlay disabled)", "color: #9146FF; font-weight: bold; font-size: 12px;");
    try {
        const data = await apiFetch('/api/current');

        if (!data || data.status === "finished" || data.status === "waiting") {
            return;
        }

        console.log(`%c[Twitch Farm Helper] 🟢 Processing Account: ${data.login || data.index}, UserCode: ${data.user_code}`, "color: #2cf6b3; font-weight: bold;");

        if (window.location.pathname.includes("/settings/connections") || window.location.pathname.includes("/settings")) {
            const currentSession = await getActiveSession();
            await setActiveSession(null);
            
            setInterval(async () => {
                try {
                    const nextData = await apiFetch('/api/current');
                    if (nextData && nextData.user_code && nextData.user_code !== currentSession) {
                        chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                            window.location.href = "https://www.twitch.tv/activate";
                        });
                    }
                } catch(e) {}
            }, 2000);
            return;
        }

        if (data.auth_token && data.user_code) {
            if (window.location.pathname === "/activate" && !window.location.search) {
                window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                return;
            }

            const activeSession = await getActiveSession();
            
            if (activeSession !== data.user_code) {
                console.log(`[Extension] 🚀 Native Cookie Wipe & Injection for Acc #${data.index}...`);
                window.localStorage.clear();
                window.sessionStorage.clear();
                
                chrome.runtime.sendMessage({ action: "WIPE_AND_INJECT", authToken: data.auth_token }, async () => {
                    await setActiveSession(data.user_code);
                    window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                });
                return;
            }

            // Start silent polling and background auto-clicker
            startPolling();
            startAutoClicker(data.password, data.login || data.index);
        }
    } catch (e) {
        // Server offline
    }
}

function startPolling() {
    const interval = setInterval(async () => {
        try {
            checkAndReportProxyBan();

            if (window.location.pathname.includes("/settings/connections") || window.location.pathname.includes("/settings")) {
                clearInterval(interval);
                const currentSession = await getActiveSession();
                await setActiveSession(null);
                
                setInterval(async () => {
                    try {
                        const nextData = await apiFetch('/api/current');
                        if (nextData && nextData.user_code && nextData.user_code !== currentSession) {
                            chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                                window.location.href = "https://www.twitch.tv/activate";
                            });
                        }
                    } catch(e) {}
                }, 2000);
                return;
            }

            let data;
            try {
                data = await apiFetch('/api/current');
            } catch (e) {
                return;
            }
            if (!data) return;
            
            // Check if we are logged out
            const domLoginElem = document.querySelector('[data-a-target="user-display-name"]');
            const loginBtn = document.querySelector('[data-a-target="login-button"]') || Array.from(document.querySelectorAll('button, a')).find(b => {
                const t = (b.innerText || "").toLowerCase().trim();
                return t === "log in" || t === "войти";
            });

            if (!domLoginElem && loginBtn) {
                console.log("[Extension] ⚠️ Auth failed or logged out! Wiping and retrying...");
                clearInterval(interval);
                window.localStorage.clear();
                window.sessionStorage.clear();
                await setActiveSession(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                });
                return;
            }

            if (data.status === "authorized" || data.status === "skipped") {
                clearInterval(interval);
                console.log(`[Extension] ✅ Account #${data.index || ''} ${data.status}! Moving next...`);
                await setActiveSession(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    window.location.href = "https://www.twitch.tv/activate";
                });
            }
        } catch (e) {
            console.error("[Extension] Poll error:", e);
        }
    }, 1500);
}

let autoclickInterval = null;
let isClickPending = false;

function startAutoClicker(password, login) {
    if (autoclickInterval) return;
    
    autoclickInterval = setInterval(() => {
        checkAndReportProxyBan();
        if (isClickPending) return;

        const findButtonByText = (texts) => {
            const buttons = Array.from(document.querySelectorAll('button, a, div[role="button"]'));
            return buttons.find(b => {
                const bTarget = (b.getAttribute("data-a-target") || "").toLowerCase();
                if (bTarget === "consent-accept-button" || bTarget === "authorize-button") return true;
                const bText = (b.innerText || b.textContent || "").toLowerCase().trim();
                return texts.some(t => bText === t.toLowerCase() || bText.includes(t.toLowerCase()));
            });
        };

        const pwdInput = document.querySelector('input[type="password"]');
        const userInput = document.querySelector('input[autocomplete="username"], input[id="login-username"]') || (pwdInput && pwdInput.form ? pwdInput.form.querySelector('input[type="text"]') : null);
        
        // Auto-fill password and submit verify
        if (pwdInput && !userInput && document.body.contains(pwdInput)) {
            if (pwdInput.value !== password && password) {
                console.log("[Auto-Clicker] 🔑 Auto-typing password for Verification...");
                pwdInput.value = password;
                pwdInput.dispatchEvent(new Event('input', { bubbles: true }));
                pwdInput.dispatchEvent(new Event('change', { bubbles: true }));
                
                isClickPending = true;
                setTimeout(() => {
                    const verifyBtn = findButtonByText(["Verify", "Подтвердить", "Confirm"]);
                    if (verifyBtn && !verifyBtn.disabled) {
                        console.log("[Auto-Clicker] 👉 Clicking Verify...");
                        verifyBtn.click();
                    }
                    isClickPending = false;
                }, Math.floor(Math.random() * 300) + 300);
            }
            return;
        }

        const activateBtn = findButtonByText(["Activate", "Активировать"]);
        if (activateBtn && !activateBtn.disabled && activateBtn.getAttribute("aria-disabled") !== "true") {
            console.log("[Auto-Clicker] 👉 Found Activate button. Clicking...");
            isClickPending = true;
            setTimeout(() => {
                if (document.body.contains(activateBtn)) {
                    activateBtn.focus();
                    activateBtn.click();
                }
                isClickPending = false;
            }, Math.floor(Math.random() * 300) + 300);
            return;
        }

        const remindBtn = findButtonByText(["Remind me later", "Напомнить позже"]);
        if (remindBtn && !remindBtn.disabled && remindBtn.getAttribute("aria-disabled") !== "true") {
            console.log("[Auto-Clicker] 👉 Clicking Remind me later...");
            isClickPending = true;
            setTimeout(() => {
                if (document.body.contains(remindBtn)) {
                    remindBtn.click();
                }
                isClickPending = false;
            }, Math.floor(Math.random() * 300) + 300);
            return;
        }

        const authBtn = findButtonByText(["Authorize", "Разрешить", "Allow", "Consent", "Подтвердить"]);
        if (authBtn) {
            window.dispatchEvent(new Event('focus', { bubbles: true }));
            document.dispatchEvent(new Event('focus', { bubbles: true }));
            
            if (!authBtn.disabled && authBtn.getAttribute("aria-disabled") !== "true") {
                console.log("[Auto-Clicker] 🎯 Found Authorize button! Triggering click...");
                isClickPending = true;
                setTimeout(() => {
                    if (document.body.contains(authBtn)) {
                        authBtn.focus();
                        authBtn.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, cancelable: true, view: window }));
                        authBtn.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, view: window }));
                        authBtn.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, view: window }));
                        authBtn.click();
                    }
                    isClickPending = false;
                }, Math.floor(Math.random() * 250) + 250);
                return;
            } else {
                authBtn.removeAttribute('disabled');
                authBtn.setAttribute('aria-disabled', 'false');
            }
        }
    }, 800);
}

handleAccountFlow();
