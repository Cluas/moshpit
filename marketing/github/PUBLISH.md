# 发布这个引流仓库（内部说明，不要推到公共仓库）

定位：**产品不开源**。这个公共仓库 = README 落地页 + 公开 issue 支持渠道 +
截图/发布说明的家。借鉴闭源 App 用 GitHub 引流的常见做法（社会证明、GitHub
搜索流量、给 awesome-list 一个可链接的地址）。

## 发布步骤（等确认后执行）

```sh
cd marketing/github
mkdir -p assets && cp ../site/assets/icon-192.png ../site/assets/40-agents-home.png ../site/assets/43-agent-breadcrumb.png assets/
cp -R ../../.github .github        # bug/feature issue 模板（已是 Moshpit 品牌）
rm -f PUBLISH.md                    # 本文件不推
git init -b main && git add -A && git commit -m "Moshpit — public home"
gh repo create Cluas/moshpit --public --source . --push \
  --description "The iOS terminal built for agent work — SSH · Mosh · tmux · herdr. Answer your agents from the lock screen."
gh repo edit Cluas/moshpit --homepage "https://moshpit.cluas.eu.org" \
  --add-topic ios --add-topic terminal --add-topic ssh --add-topic mosh \
  --add-topic tmux --add-topic claude-code --add-topic ai-agents
# Discussions 在 repo Settings 里手动开
```

发布后顺手做的引流位：

1. ~~官网 footer 加 GitHub 链接~~ —— 已经在 `marketing/site-next/src/layouts/Marketing.astro`
   的页脚里了，一处改动覆盖全部页面。改完跑 `scripts/deploy-site.sh`（configmap
   那套早就退役了）。
2. 提交 awesome-lists：awesome-ios、awesome-tmux、awesome-claude-code（各自开 PR）
3. HN "Show HN" / X 首发帖正文放 GitHub 链接（比裸官网更容易被点）

## 注意

- ⚠️ 私有产品仓库根目录有 **MIT LICENSE**（开源时代遗留）。既然定了不开源：
  私有仓库**不要 public**；这个引流仓库不放 LICENSE（README 明说 not open source）。
  要不要把私有仓库的 LICENSE 换成 proprietary 声明，等用户拍板。
- scripts/moshpit-notify.sh 是带占位 API 的遗留脚本，**不要**放进公共仓库，免得误导。
  （注：「不做远程推送」这个结论已在 2026-08-23 推翻 —— 真的推送链路见 `docs/PUSH.md`
  和 `push-relay/`，那个占位脚本是被它取代、而不是仍然代表产品方向。）
