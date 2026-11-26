(() => {
  document.documentElement.classList.add("js");

  const stateLabels = {
    not_started: "未开始",
    running: "进行中",
    ended: "已结束",
  };

  function formatSeconds(total) {
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  function redirectForStatus(status, contestId) {
    if (status === 401) {
      window.location.href = `/c/${contestId}/join?reason=invalid`;
      return true;
    }
    if (status === 403) {
      window.location.href = `/c/${contestId}`;
      return true;
    }
    return false;
  }

  function initCountdown() {
    const element = document.querySelector("[data-countdown]");
    if (!element) {
      return;
    }
    const contestId = element.dataset.contest;
    const stateElement = document.querySelector("[data-state]");
    let remaining = Number(element.dataset.seconds || "0");

    const render = () => {
      element.textContent = formatSeconds(Math.max(0, remaining));
    };
    render();

    window.setInterval(() => {
      if (remaining > 0) {
        remaining -= 1;
        render();
      }
    }, 1000);

    const poll = async () => {
      try {
        const response = await fetch(`/c/${contestId}/status`, {
          headers: { Accept: "application/json" },
        });
        if (redirectForStatus(response.status, contestId)) {
          return;
        }
        if (!response.ok) {
          return;
        }
        const data = await response.json();
        remaining = Number(data.remainingSeconds || 0);
        render();
        if (stateElement) {
          stateElement.textContent = stateLabels[data.state] || data.state;
        }
      } catch (_) {
        // 保留旧倒计时，下一轮继续重试。
      }
    };

    poll();
    window.setInterval(poll, 2000);
  }

  function initTabs() {
    const root = document.querySelector("[data-tabs]");
    if (!root) {
      return;
    }
    const buttons = Array.from(root.querySelectorAll("[data-tab]"));
    const panels = Array.from(root.querySelectorAll("[data-tab-panel]"));
    if (buttons.length === 0 || panels.length === 0) {
      return;
    }

    const activate = (name, remember) => {
      buttons.forEach((button) => {
        const active = button.dataset.tab === name;
        button.classList.toggle("active", active);
        button.setAttribute("aria-selected", active ? "true" : "false");
      });
      panels.forEach((panel) => {
        panel.hidden = panel.dataset.tabPanel !== name;
      });
      if (remember) {
        history.replaceState(null, "", `#${name}`);
      }
      // 刚显示的标签里如果有轮询片段，立即拉一次，不用等下一个周期。
      refreshVisibleFragments();
    };

    buttons.forEach((button) => {
      button.addEventListener("click", () => activate(button.dataset.tab, true));
    });

    const fromHash = window.location.hash.replace("#", "");
    const initial = buttons.some((button) => button.dataset.tab === fromHash)
      ? fromHash
      : buttons[0].dataset.tab;
    activate(initial, false);

    // 同页内跳到 #me / #rank 时（例如题目页返回链接）也能切换标签。
    window.addEventListener("hashchange", () => {
      const name = window.location.hash.replace("#", "");
      if (buttons.some((button) => button.dataset.tab === name)) {
        activate(name, false);
      }
    });
  }

  function initWaiting() {
    const element = document.querySelector("[data-waiting]");
    if (!element) {
      return;
    }
    const contestId = element.dataset.contest;
    let reloading = false;
    window.setInterval(async () => {
      if (reloading) {
        return;
      }
      try {
        const response = await fetch(`/c/${contestId}/status`, {
          headers: { Accept: "application/json" },
        });
        if (redirectForStatus(response.status, contestId)) {
          return;
        }
        if (!response.ok) {
          return;
        }
        const data = await response.json();
        if (data.state && data.state !== "not_started") {
          reloading = true;
          window.location.reload();
        }
      } catch (_) {
        // 网络抖动时保持等待页，下一轮继续探测。
      }
    }, 3000);
  }

  // 片段轮询会用新 HTML 覆盖旧内容，浏览器会把 <details> 全部收回。
  // 刷新前记下展开项（按 data-key 标识），写入新内容后再恢复。
  const fragmentRefreshers = [];

  function collectOpenKeys(root) {
    const keys = new Set();
    root.querySelectorAll("details[data-key]").forEach((element) => {
      if (element.open) {
        keys.add(element.dataset.key);
      }
    });
    return keys;
  }

  function restoreOpenKeys(root, keys) {
    if (keys.size === 0) {
      return;
    }
    root.querySelectorAll("details[data-key]").forEach((element) => {
      if (keys.has(element.dataset.key)) {
        element.open = true;
      }
    });
  }

  function initFragments() {
    document.querySelectorAll("[data-fragment-url]").forEach((element) => {
      const url = element.dataset.fragmentUrl;
      const interval = Number(element.dataset.interval || "2000");
      const content = element.querySelector(".fragment-content");
      const error = element.querySelector("[data-poll-error]");
      const contestMatch = url.match(/^\/c\/([^/]+)\//);
      const contestId = contestMatch ? contestMatch[1] : null;
      let lastHtml = null;

      const refresh = async () => {
        // 隐藏的标签页不轮询，切回时服务端渲染的内容仍然在。
        if (element.closest("[hidden]")) {
          return;
        }
        try {
          const response = await fetch(url, { headers: { Accept: "text/html" } });
          if (contestId && redirectForStatus(response.status, contestId)) {
            return;
          }
          if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
          }
          const html = await response.text();
          if (html === lastHtml) {
            return;
          }
          lastHtml = html;
          const openKeys = collectOpenKeys(content);
          content.innerHTML = html;
          restoreOpenKeys(content, openKeys);
          if (error) {
            error.hidden = true;
          }
        } catch (_) {
          if (error) {
            error.hidden = false;
          }
        }
      };

      fragmentRefreshers.push(refresh);
      window.setInterval(refresh, interval);
    });
  }

  function refreshVisibleFragments() {
    fragmentRefreshers.forEach((refresh) => refresh());
  }

  function initEditor() {
    const textarea = document.querySelector("#editor");
    if (!textarea || !window.CodeMirror) {
      return;
    }
    const contest = textarea.dataset.contest;
    const student = textarea.dataset.student;
    const problem = textarea.dataset.problem;
    const draftKey = `lc:draft:${contest}:${student}:${problem}`;

    const editor = window.CodeMirror.fromTextArea(textarea, {
      lineNumbers: true,
      mode: "text/x-c++src",
      indentUnit: 4,
      tabSize: 4,
      extraKeys: {
        Tab: (cm) => cm.replaceSelection("    "),
      },
    });

    const saved = window.localStorage.getItem(draftKey);
    if (saved !== null) {
      editor.setValue(saved);
    }

    let timer = null;
    editor.on("change", () => {
      if (timer !== null) {
        window.clearTimeout(timer);
      }
      timer = window.setTimeout(() => {
        window.localStorage.setItem(draftKey, editor.getValue());
      }, 300);
    });
  }

  // 提交后跳回题目页会带 ?submitted=1，刷新时把参数去掉，避免旧提示反复出现。
  function initNotice() {
    if (!document.querySelector("[data-submitted-notice]")) {
      return;
    }
    const url = new URL(window.location.href);
    if (url.searchParams.get("submitted") !== "1") {
      return;
    }
    url.searchParams.delete("submitted");
    const search = url.searchParams.toString();
    history.replaceState(null, "", `${url.pathname}${search ? `?${search}` : ""}${url.hash}`);
  }

  function init() {
    initCountdown();
    initFragments();
    initTabs();
    initWaiting();
    initEditor();
    initNotice();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
