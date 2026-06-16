"use strict";

const MARKS = { "◎": "honmei", "○": "taikou", "▲": "tanana", "△": "renge", "☆": "hoshi", "✓": "check" };

// 8 桁日付を読みやすく整形。それ以外はそのまま。
function prettify(name) {
  const m = /^(\d{4})(\d{2})(\d{2})$/.exec(name);
  return m ? `${m[1]}-${m[2]}-${m[3]}` : name;
}

function renderTree(nodes, parent) {
  const ul = document.createElement("ul");
  for (const n of nodes) {
    const li = document.createElement("li");
    if (n.path) {
      const a = document.createElement("a");
      a.className = "file";
      a.href = "#";
      a.textContent = n.name;
      a.dataset.path = n.path;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        selectDoc(n.path, a);
      });
      li.appendChild(a);
    } else {
      const head = document.createElement("div");
      head.className = "dir";
      head.textContent = prettify(n.name);
      head.addEventListener("click", () => li.classList.toggle("collapsed"));
      const wrap = document.createElement("div");
      wrap.className = "children";
      renderTree(n.children, wrap);
      li.appendChild(head);
      li.appendChild(wrap);
    }
    ul.appendChild(li);
  }
  parent.appendChild(ul);
}

// テーブルセルを数値判定して右寄せ、印セルを色付け。
function enhance(root) {
  for (const cell of root.querySelectorAll("td, th")) {
    const t = cell.textContent.trim();
    if (/\d/.test(t) && /^[¥()%+\-.,\d\s]+$/.test(t)) {
      cell.classList.add("num");
    }
    const mark = MARKS[t];
    if (mark) {
      cell.classList.add("mark", "mark-" + mark);
    }
  }
}

async function selectDoc(path, link) {
  const content = document.getElementById("content");
  content.innerHTML = '<p class="placeholder">読み込み中…</p>';
  document.querySelectorAll("#tree a.file.active").forEach((el) => el.classList.remove("active"));
  if (link) link.classList.add("active");
  try {
    const res = await fetch("/api/doc?path=" + encodeURIComponent(path));
    if (!res.ok) {
      content.innerHTML = `<p class="error">読み込み失敗 (${res.status})</p>`;
      return;
    }
    const html = await res.text();
    const article = document.createElement("article");
    article.className = "md";
    article.innerHTML = html;
    enhance(article);
    content.innerHTML = "";
    content.appendChild(article);
    content.scrollTop = 0;
  } catch (err) {
    content.innerHTML = `<p class="error">読み込みエラー: ${err}</p>`;
  }
}

async function init() {
  const tree = document.getElementById("tree");
  try {
    const res = await fetch("/api/tree");
    const nodes = await res.json();
    tree.innerHTML = "";
    if (!nodes.length) {
      tree.innerHTML = '<p class="placeholder">予想ファイルが見つかりません。</p>';
      return;
    }
    renderTree(nodes, tree);
  } catch (err) {
    tree.innerHTML = `<p class="error">ツリー取得エラー: ${err}</p>`;
  }
}

init();
