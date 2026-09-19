const SERVER_URL = "http://127.0.0.1:5000";

// Listen for storage wipe commands from background service worker
if (chrome && chrome.runtime && chrome.runtime.onMessage) {
    chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
        if (request && request.action === "WIPE_LOCAL_STORAGE") {
            try {
                localStorage.clear();
                sessionStorage.clear();
                const panel = document.getElementById("farm-helper-panel");
                if (panel) panel.remove();
                console.log("%c[Content] 🧹 LocalStorage and SessionStorage cleared on queue completion", "color: #ff4757; font-weight: bold;");
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

function getActiveAccount() {
    return new Promise(resolve => chrome.storage.local.get(['active_account_login'], res => resolve(res.active_account_login)));
}

function setActiveAccount(val) {
    return new Promise(resolve => {
        if (val === null) chrome.storage.local.remove('active_account_login', resolve);
        else chrome.storage.local.set({active_account_login: String(val)}, resolve);
    });
}

function renderFloatingPanel(login, password) {
    if (document.getElementById("farm-helper-panel")) return;
    if (!document.body) {
        setTimeout(() => renderFloatingPanel(login, password), 200);
        return;
    }

    const savedLeft = localStorage.getItem("farm_panel_left");
    const savedTop = localStorage.getItem("farm_panel_top");

    const panel = document.createElement("div");
    panel.id = "farm-helper-panel";
    
    let positionCss = `top: 20px; right: 20px;`;
    if (savedLeft && savedTop) {
        positionCss = `top: ${savedTop}; left: ${savedLeft}; right: auto;`;
    }

    panel.style.cssText = `
        position: fixed;
        ${positionCss}
        z-index: 999999;
        background: #18181b;
        color: #ffffff;
        border: 2px solid #9146FF;
        border-radius: 12px;
        padding: 12px;
        font-family: Inter, Roboto, sans-serif;
        box-shadow: 0px 4px 15px rgba(0,0,0,0.5);
    `;

    panel.innerHTML = `
        <div id="farm-panel-header" style="font-weight: bold; color: #9146FF; font-size: 15px; margin-bottom: 8px; display: flex; align-items: center; justify-content: space-between; cursor: move; padding-bottom: 4px; border-bottom: 1px solid #26262c;">
            <span>✋ 🎮 Twitch Android OAuth Helper</span>
            <span id="farm-acc-title" style="background: #9146FF; color: #fff; font-size: 11px; padding: 2px 8px; border-radius: 10px;">${login || 'Acc'}</span>
        </div>
        <div style="font-size: 12px; color: #adadb8; margin-top: 8px; margin-bottom: 6px;">Пароль от этого аккаунта:</div>
        <div style="display: flex; gap: 8px; margin-bottom: 10px;">
            <input type="text" id="farm-password-input" value="${password || 'Пароль не указан'}" readonly style="background: #0e0e10; border: 1px solid #464649; color: #efeff1; padding: 8px 12px; border-radius: 6px; flex: 1; font-family: monospace; font-size: 14px; font-weight: bold;" />
            <button id="farm-copy-btn" style="background: #9146FF; color: #ffffff; border: none; padding: 8px 14px; border-radius: 6px; font-weight: bold; cursor: pointer; font-size: 13px; transition: 0.2s;">Скопировать</button>
        </div>
        <div style="display: flex; gap: 8px; border-top: 1px solid #26262c; padding-top: 10px; margin-bottom: 8px;">
            <button id="farm-autoclick-btn" style="background: #464649; color: #ffffff; border: none; padding: 8px 12px; border-radius: 6px; font-weight: bold; cursor: pointer; font-size: 12px; flex: 1; transition: 0.2s;">
                🤖 Авто-клик: ВЫКЛ
            </button>
        </div>
        <div style="display: flex; gap: 8px; border-top: 1px solid #26262c; padding-top: 10px;">
            <button id="farm-skip-btn" style="background: #eb0400; color: #ffffff; border: none; padding: 8px 12px; border-radius: 6px; font-weight: bold; cursor: pointer; font-size: 12px; flex: 1; transition: 0.2s;">
                ⏭️ Скипнуть этот аккаунт
            </button>
        </div>
        <div id="farm-status-msg" style="font-size: 12px; color: #2cf6b3; margin-top: 8px; text-align: center; font-weight: bold; display: none;"></div>
        <div style="font-size: 11px; color: #adadb8; margin-top: 8px; text-align: center;">
            💡 <i>Зажми заголовок, чтобы перетащить карточку!</i>
        </div>
    `;

    document.body.appendChild(panel);

    // Draggable
    const header = document.getElementById("farm-panel-header");
    let isDragging = false;
    let startX = 0, startY = 0, initialLeft = 0, initialTop = 0;

    header.onmousedown = (e) => {
        isDragging = true;
        startX = e.clientX;
        startY = e.clientY;
        const rect = panel.getBoundingClientRect();
        initialLeft = rect.left;
        initialTop = rect.top;
        panel.style.right = "auto";

        const onMouseMove = (moveEvt) => {
            if (!isDragging) return;
            const dx = moveEvt.clientX - startX;
            const dy = moveEvt.clientY - startY;
            const newLeft = Math.max(0, Math.min(window.innerWidth - panel.offsetWidth, initialLeft + dx));
            const newTop = Math.max(0, Math.min(window.innerHeight - panel.offsetHeight, initialTop + dy));
            panel.style.left = `${newLeft}px`;
            panel.style.top = `${newTop}px`;
        };

        const onMouseUp = () => {
            if (!isDragging) return;
            isDragging = false;
            document.removeEventListener("mousemove", onMouseMove);
            document.removeEventListener("mouseup", onMouseUp);
            localStorage.setItem("farm_panel_left", panel.style.left);
            localStorage.setItem("farm_panel_top", panel.style.top);
        };

        document.addEventListener("mousemove", onMouseMove);
        document.addEventListener("mouseup", onMouseUp);
    };

    const copyBtn = document.getElementById("farm-copy-btn");
    const pwdInput = document.getElementById("farm-password-input");
    const skipBtn = document.getElementById("farm-skip-btn");

    const doCopy = () => {
        pwdInput.select();
        navigator.clipboard.writeText(pwdInput.value);
        copyBtn.innerText = "Скопировано!";
        copyBtn.style.background = "#2cf6b3";
        copyBtn.style.color = "#000";
        setTimeout(() => {
            copyBtn.innerText = "Скопировать";
            copyBtn.style.background = "#9146FF";
            copyBtn.style.color = "#fff";
        }, 2000);
    };

    pwdInput.onclick = doCopy;
    copyBtn.onclick = doCopy;

    const autoclickBtn = document.getElementById("farm-autoclick-btn");
    let isAutoClick = localStorage.getItem("farm_autoclick") !== "false";
    
    const updateAutoClickBtn = () => {
        if (isAutoClick) {
            autoclickBtn.innerText = "🤖 Авто-клик: ВКЛ";
            autoclickBtn.style.background = "#2cf6b3";
            autoclickBtn.style.color = "#000";
        } else {
            autoclickBtn.innerText = "🤖 Авто-клик: ВЫКЛ";
            autoclickBtn.style.background = "#464649";
            autoclickBtn.style.color = "#fff";
        }
    };
    updateAutoClickBtn();

    autoclickBtn.onclick = () => {
        isAutoClick = !isAutoClick;
        localStorage.setItem("farm_autoclick", isAutoClick);
        updateAutoClickBtn();
    };

    skipBtn.onclick = async () => {
        if (confirm(`Пропустить аккаунт ${login} и перейти к следующему?`)) {
            skipBtn.innerText = "Пропускаем...";
            skipBtn.disabled = true;
            try {
                await apiFetch('/api/skip', 'POST');
                await setActiveAccount(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    window.location.reload();
                });
            } catch (e) {
                alert("Ошибка пропуска аккаунта: " + e);
            }
        }
    };
}

async function checkAndSendAccessToken(currentLogin) {
    if (window.location.hash && window.location.hash.includes("access_token=")) {
        const hash = window.location.hash.substring(1);
        const params = new URLSearchParams(hash);
        const accessToken = params.get("access_token");

        if (accessToken) {
            console.log("%c[Extension] 🔑 Access token detected in URL hash!", "color: #2cf6b3; font-weight: bold; font-size: 14px;");
            
            const statusElem = document.getElementById("farm-status-msg");
            if (statusElem) {
                statusElem.style.display = "block";
                statusElem.innerText = "✅ Токен получен! Отправляем на сервер...";
            }

            chrome.runtime.sendMessage({
                action: "SEND_TOKEN",
                accessToken: accessToken,
                login: currentLogin || ""
            }, (response) => {
                console.log("[Extension] Token response from server:", response);
                if (statusElem) {
                    statusElem.innerText = "✨ Токен сохранён! Переходим к следующему...";
                }
                history.replaceState(null, null, window.location.pathname + window.location.search);
            });
            return true;
        }
    }
    return false;
}

async function handleAccountFlow() {
    console.log("%c[Twitch Farm Helper] 🚀 Extension active on page (Android OAuth)", "color: #9146FF; font-weight: bold; font-size: 12px;");
    try {
        const data = await apiFetch('/api/current');

        if (!data || data.status === "finished" || data.status === "waiting") {
            console.log("[Twitch Farm Helper] ℹ️ Auth server not active");
            return;
        }

        const currentLogin = data.login || data.index;
        console.log(`%c[Twitch Farm Helper] 🟢 Auth Server Connected! Account: ${currentLogin}, Status: ${data.status}`, "color: #2cf6b3; font-weight: bold; font-size: 13px;");

        // 1. Check if we just received access_token in the redirect URL hash!
        const tokenSent = await checkAndSendAccessToken(currentLogin);
        if (tokenSent) {
            return;
        }

        if (data.status === "authorized") {
            console.log(`[Extension] ✅ Account ${currentLogin} authorized! Wiping session and preparing next...`);
            await setActiveAccount(null);
            chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                setTimeout(() => window.location.reload(), 1500);
            });
            return;
        }

        if (data.auth_token && data.auth_url) {
            const activeAcc = await getActiveAccount();

            // New account in queue
            if (activeAcc !== currentLogin) {
                console.log(`[Extension] 🚀 Native HttpOnly Cookie Wipe & Setup for ${currentLogin}...`);
                
                const acState = window.localStorage.getItem("farm_autoclick");
                const panelL = window.localStorage.getItem("farm_panel_left");
                const panelT = window.localStorage.getItem("farm_panel_top");
                
                window.localStorage.clear();
                window.sessionStorage.clear();
                
                if (acState !== null) window.localStorage.setItem("farm_autoclick", acState);
                if (panelL !== null) window.localStorage.setItem("farm_panel_left", panelL);
                if (panelT !== null) window.localStorage.setItem("farm_panel_top", panelT);
                
                chrome.runtime.sendMessage({ action: "WIPE_AND_INJECT", authToken: data.auth_token }, async () => {
                    await setActiveAccount(currentLogin);
                    window.location.href = data.auth_url;
                });
                return;
            }

            renderFloatingPanel(currentLogin, data.password);
            startPolling(currentLogin);
            startAutoClicker(data.password, currentLogin);
        }
    } catch (e) {
        // Server offline
    }
}

function startPolling(currentLogin) {
    const interval = setInterval(async () => {
        try {
            // Also check for hash in SPA transitions
            const tokenSent = await checkAndSendAccessToken(currentLogin);
            if (tokenSent) {
                clearInterval(interval);
                return;
            }

            let data;
            try {
                data = await apiFetch('/api/current');
            } catch (e) {
                return;
            }
            if (!data) return;

            if (data.status === "authorized") {
                clearInterval(interval);
                console.log(`[Extension] ✅ Account ${currentLogin} authorized! Moving next...`);
                await setActiveAccount(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    setTimeout(() => window.location.reload(), 1000);
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
        if (window.localStorage.getItem("farm_autoclick") === "false") return;
        if (isClickPending) return;

        const findButtonByText = (texts) => {
            const buttons = Array.from(document.querySelectorAll('button, a, div[role="button"], input[type="submit"]'));
            return buttons.find(b => {
                const bTarget = (b.getAttribute("data-a-target") || "").toLowerCase();
                if (
                    bTarget === "passport-oauth-authorize-button" || 
                    bTarget === "authorize-button" || 
                    bTarget.includes("authorize") || 
                    bTarget === "consent-accept-button"
                ) return true;
                const bText = (b.innerText || b.value || b.textContent || "").toLowerCase().trim();
                return texts.some(t => bText === t.toLowerCase() || bText.includes(t.toLowerCase()));
            });
        };

        // 1. Check for Password input (if Twitch prompts for password confirmation)
        const pwdInput = document.querySelector('input[type="password"]');
        const userInput = document.querySelector('input[autocomplete="username"], input[id="login-username"]') || (pwdInput && pwdInput.form ? pwdInput.form.querySelector('input[type="text"]') : null);
        
        if (pwdInput && !userInput && document.body.contains(pwdInput)) {
            if (pwdInput.value !== password && password) {
                console.log("[Auto-Clicker] 🔑 Typing password for Verification...");
                pwdInput.value = password;
                pwdInput.dispatchEvent(new Event('input', { bubbles: true }));
                pwdInput.dispatchEvent(new Event('change', { bubbles: true }));
                
                isClickPending = true;
                setTimeout(() => {
                    const verifyBtn = findButtonByText(["Verify", "Подтвердить", "Confirm", "Log In", "Войти"]);
                    if (verifyBtn && !verifyBtn.disabled) {
                        console.log("[Auto-Clicker] 👉 Clicking Verify...");
                        verifyBtn.click();
                    }
                    isClickPending = false;
                }, 500);
            }
            return;
        }

        // 2. Check for OAuth Authorize / Connect button
        const authBtn = findButtonByText([
            "Authorize", "Разрешить", "Accept", "Connect", "Подтвердить", "Allow"
        ]);
        if (authBtn && !authBtn.disabled && authBtn.getAttribute("aria-disabled") !== "true") {
            console.log("[Auto-Clicker] 👉 Found OAuth Authorize button. Clicking...", authBtn);
            isClickPending = true;
            setTimeout(() => {
                authBtn.click();
                isClickPending = false;
            }, 600);
            return;
        }
    }, 1000);
}

// Kick off when DOM is ready
if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", handleAccountFlow);
} else {
    handleAccountFlow();
}
