const SERVER_URL = "http://localhost:5000";

function apiFetch(endpoint) {
    return new Promise((resolve, reject) => {
        if (!chrome || !chrome.runtime || !chrome.runtime.sendMessage) {
            return reject(new Error("Extension context invalidated"));
        }
        try {
            chrome.runtime.sendMessage({ action: "FETCH_API", endpoint }, (response) => {
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

function renderFloatingPanel(index, password) {
    if (document.getElementById("farm-helper-panel")) return;
    if (!document.body) {
        setTimeout(() => renderFloatingPanel(index, password), 200);
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

    function getTwitchUsername() {
        try {
            const match = document.cookie.match(/(?:^|;\s*)twilight-user=([^;]+)/);
            if (match) {
                const json = JSON.parse(decodeURIComponent(match[1]));
                if (json && json.displayName) return json.displayName;
                if (json && json.login) return json.login;
            }
        } catch(e) {}
        return null;
    }

    let displayLogin = index;
    if (String(index) === "0") {
        displayLogin = getTwitchUsername() || "0";
    }

    panel.innerHTML = `
        <div id="farm-panel-header" style="font-weight: bold; color: #9146FF; font-size: 15px; margin-bottom: 8px; display: flex; align-items: center; justify-content: space-between; cursor: move; padding-bottom: 4px; border-bottom: 1px solid #26262c;">
            <span>✋ 🎮 Twitch Extension Helper</span>
            <span id="farm-acc-title" style="background: #9146FF; color: #fff; font-size: 11px; padding: 2px 8px; border-radius: 10px;">Acc #${displayLogin}</span>
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
        <div style="font-size: 11px; color: #adadb8; margin-top: 8px; text-align: center;">
            💡 <i>Зажми заголовок, чтобы перетащить карточку!</i>
        </div>
    `;

    document.body.appendChild(panel);

    // Make floating panel Draggable by dragging the header
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
    // Default to true if not explicitly "false"
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
        if (confirm(`Пропустить аккаунт #${index} и перейти к следующему?`)) {
            skipBtn.innerText = "Пропускаем...";
            skipBtn.disabled = true;
            try {
                await apiFetch('/api/skip');
                await setActiveSession(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    window.location.href = "https://www.twitch.tv/activate";
                });
            } catch (e) {
                alert("Ошибка пропуска аккаунта: " + e);
            }
        }
    };
}

async function handleAccountFlow() {
    console.log("%c[Twitch Farm Helper] 🚀 Extension active on page", "color: #9146FF; font-weight: bold; font-size: 12px;");
    try {
        // Check if server is active FIRST before doing any redirects or script execution
        const data = await apiFetch('/api/current');

        if (!data || data.status === "finished" || data.status === "waiting") {
            console.log("[Twitch Farm Helper] ℹ️ Auth server not active (waiting for worker queue...)");
            return;
        }

        console.log(`%c[Twitch Farm Helper] 🟢 Auth Server Connected! Account: ${data.login || data.index}, UserCode: ${data.user_code}`, "color: #2cf6b3; font-weight: bold; font-size: 13px;");
        if (data.proxy) {
            console.log(`[Twitch Farm Helper] 🌐 Proxy configured via bridge (${data.proxy.protocol || 'http'}://${data.proxy.host}:${data.proxy.port})`);
        }

        // Only redirect when server is running and processing an account
        if (window.location.pathname.includes("/settings/connections") || window.location.pathname.includes("/settings")) {
            console.log("[Extension] 🎯 Landed on settings page! Waiting for PowerShell to advance to the next account...");
            
            const currentSession = await getActiveSession();
            await setActiveSession(null);
            
            // We must wait for the old C# container to be killed and the new one to start
            setInterval(async () => {
                try {
                    const nextData = await apiFetch('/api/current');
                    if (nextData && nextData.user_code && nextData.user_code !== currentSession) {
                        console.log("[Extension] New container detected! Proceeding...");
                        chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                            window.location.href = "https://www.twitch.tv/activate";
                        });
                    }
                } catch(e) {
                    // Server is offline (container killed by PS script). Just wait.
                }
            }, 2000);
            return;
        }

        if (data.auth_token && data.user_code) {
            
            // If we somehow landed on /activate without the device-code in the URL, force a redirect!
            if (window.location.pathname === "/activate" && !window.location.search) {
                window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                return;
            }

            const activeSession = await getActiveSession();
            
            // If the user_code is different, it's a new auth session
            if (activeSession !== data.user_code) {
                console.log(`[Extension] 🚀 Native HttpOnly Cookie Wipe & Setup for Acc #${data.index}...`);
                
                // Back up extension state
                const acState = window.localStorage.getItem("farm_autoclick");
                const panelL = window.localStorage.getItem("farm_panel_left");
                const panelT = window.localStorage.getItem("farm_panel_top");
                
                // CRITICAL: Clear localStorage before wiping so Twitch doesn't detect a session mismatch!
                window.localStorage.clear();
                window.sessionStorage.clear();
                
                // Restore extension state
                if (acState !== null) window.localStorage.setItem("farm_autoclick", acState);
                if (panelL !== null) window.localStorage.setItem("farm_panel_left", panelL);
                if (panelT !== null) window.localStorage.setItem("farm_panel_top", panelT);
                
                chrome.runtime.sendMessage({ action: "WIPE_AND_INJECT", authToken: data.auth_token }, async () => {
                    await setActiveSession(data.user_code);
                    window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                });
                return;
            }

            renderFloatingPanel(data.login || data.index, data.password);
            startPolling();
            startAutoClicker(data.password, data.login || data.index);
        }
    } catch (e) {
        // Server is offline: do nothing!
    }
}

function startPolling() {
    const interval = setInterval(async () => {
        try {
            if (window.location.pathname.includes("/settings/connections") || window.location.pathname.includes("/settings")) {
                clearInterval(interval);
                console.log("[Extension] 🎯 Landed on settings page (via SPA)! Waiting for PowerShell to advance to the next account...");
                
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
                    } catch(e) {
                        // wait
                    }
                }, 2000);
                return;
            }

            let data;
            try {
                data = await apiFetch('/api/current');
            } catch (e) {
                // Server offline / restarting
                return;
            }
            if (!data) return;
            
            // 1. Check if we are fully logged in
            const domLoginElem = document.querySelector('[data-a-target="user-display-name"]');
            
            // Use robust data attribute for login button, fallback to text
            const loginBtn = document.querySelector('[data-a-target="login-button"]') || Array.from(document.querySelectorAll('button, a')).find(b => {
                const t = (b.innerText || "").toLowerCase().trim();
                return t === "log in" || t === "войти";
            });

            // If we see a login button and no user name, we are logged out.
            // (We cannot check expectedLogin because the C# server doesn't provide it reliably)
            if (!domLoginElem && loginBtn) {
                console.log("[Extension] ⚠️ Auth failed or logged out! Wiping and retrying...");
                clearInterval(interval);
                
                const acState = window.localStorage.getItem("farm_autoclick");
                const panelL = window.localStorage.getItem("farm_panel_left");
                const panelT = window.localStorage.getItem("farm_panel_top");
                
                window.localStorage.clear();
                window.sessionStorage.clear();
                
                if (acState !== null) window.localStorage.setItem("farm_autoclick", acState);
                if (panelL !== null) window.localStorage.setItem("farm_panel_left", panelL);
                if (panelT !== null) window.localStorage.setItem("farm_panel_top", panelT);
                
                await setActiveSession(null);
                chrome.runtime.sendMessage({ action: "WIPE_ONLY" }, () => {
                    window.location.href = `https://www.twitch.tv/activate?device-code=${data.user_code}`;
                });
                return;
            }

            // Fix Acc #0 display by pulling from cookie if not updated yet
            const titleElem = document.getElementById('farm-acc-title');
            if (titleElem && titleElem.innerText.includes("Acc #0")) {
                const match = document.cookie.match(/(?:^|;\s*)twilight-user=([^;]+)/);
                if (match) {
                    try {
                        const json = JSON.parse(decodeURIComponent(match[1]));
                        if (json && (json.displayName || json.login)) {
                            titleElem.innerText = `Acc #${json.displayName || json.login}`;
                        }
                    } catch(e) {}
                }
            }

            // 2. Standard polling logic
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
    }, 2000);
}

let autoclickInterval = null;
let isClickPending = false;

function startAutoClicker(password, login) {
    if (autoclickInterval) return;
    
    autoclickInterval = setInterval(() => {
        if (window.localStorage.getItem("farm_autoclick") === "false") return;
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
        
        if (pwdInput && !userInput && document.body.contains(pwdInput)) {
            if (pwdInput.value !== password && password) {
                console.log("[Auto-Clicker] 🔑 Typing password for Verification...");
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
                }, Math.floor(Math.random() * 400) + 400);
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
            }, Math.floor(Math.random() * 400) + 400);
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
            }, Math.floor(Math.random() * 400) + 400);
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
                }, Math.floor(Math.random() * 300) + 300);
                return;
            } else {
                console.log("[Auto-Clicker] Authorize button is disabled, enabling...");
                authBtn.removeAttribute('disabled');
                authBtn.setAttribute('aria-disabled', 'false');
            }
        }
    }, 1000);
}

handleAccountFlow();
