# OpenFab × agent-chat × Robrix 完整验收 Checklist

> v2 · 2026-07-06。整合实测踩坑:每条"失败排查"都对应一次真实事故。
> 角色分工:Robrix = 人的驾驶舱(纯 Matrix 客户端);agent-chat = 执行层(多 agent + Matrix relay);OpenFab = 可选的验证/签名/信任门层(默认 gate=none,机器认证不阻塞)。

---

## 阶段 0:冷启动(机器重启后从零拉起)

**三条铁律**(每条都炸过):

1. tmux 必须 `-c` 指定目录(不继承你 shell 的 cwd);
2. backend / dashboard / relay 必须 `source .env`(缺 `API_TOKEN` 秒退);
3. **backend 最先起**——relay 重试 5 次即 FATAL 退出、dashboard 的 backend-SSE 会静默挂死、桥的 registerSelf 只试一次;
4. 三个 node 服务必须 **`env -u TMUX`**——跑在 tmux 里会继承 `$TMUX`,pane 探测会扫到错误的 tmux 服务器,把默认 socket 上的 agent 周期性标死(dashboard "no known agents" 的根因)。

```bash
AC=~/Work/Projects/consult/agent-chat
OF=~/Work/Projects/FW/openfab

# ① backend 最先
tmux -L agentchat-services new -d -s backend -c "$AC" 'bash -lc "set -a; source .env; set +a; env -u TMUX node backend-v2.js >> .demo-logs/backend.log 2>&1"'
sleep 6   # 等它就绪,后面全依赖它

# ② 其余基础设施
tmux -L agentchat-services new -d -s dashboard -c "$AC" 'bash -lc "set -a; source .env; set +a; env -u TMUX node server.js >> .demo-logs/dashboard.log 2>&1"'
tmux -L agentchat-services new -d -s relay     -c "$AC" 'bash -lc "set -a; source .env; set +a; env -u TMUX node bridge-matrix.js >> .demo-logs/relay.log 2>&1"'
tmux -L agentchat-services new -d -s openfab   -c "$OF" "bash -lc 'OPENFAB_AGENTCHAT_URL=http://127.0.0.1:8077 env -u ANTHROPIC_API_KEY ./target/release/openfab serve --repo demo/.work/web --port 8787 --policy policy/trust.json >> /tmp/openfab-serve.log 2>&1'"
tmux -L agentchat-services new -d -s ofbridge  -c "$OF" "bash -lc 'AGENTCHAT_DIR=$AC node bridge/openfab-agentchat-bridge.mjs >> /tmp/openfab-bridge.log 2>&1'"

# ③ agent 进程(独立于基础设施,必须单独拉!名字保持 wf_*——Matrix 分身/群/技能路由都绑在名字上,别新造名)
cd "$AC"
# 方案 A:全 codex(走 OpenAI 额度,Claude 零花费)
./bin/agent-chat up wf_coordinator    ~/.agentchat/agents/agent_wf_coordinator/workdir    codex --fresh
./bin/agent-chat up wf_implementer    ~/.agentchat/agents/agent_wf_implementer/workdir    codex --fresh
./bin/agent-chat up wf_reviewer       ~/.agentchat/agents/agent_wf_reviewer/workdir       codex --fresh
./bin/agent-chat up wf_final_reviewer ~/.agentchat/agents/agent_wf_final_reviewer/workdir codex --fresh
# 方案 B:claude 干活 + codex 终审(异构制衡,原设计;claude 务必 --model sonnet 控成本,
# 否则会继承你 /model 设的默认——设过 Fable/Opus 会非常贵)
#   ./bin/agent-chat up wf_coordinator ~/.agentchat/agents/agent_wf_coordinator/workdir claude --fresh --model sonnet
#   ./bin/agent-chat up wf_implementer ~/.agentchat/agents/agent_wf_implementer/workdir claude --fresh --model sonnet
#   ./bin/agent-chat up wf_reviewer    ~/.agentchat/agents/agent_wf_reviewer/workdir    claude --fresh --model sonnet
#   ./bin/agent-chat up wf_final_reviewer ~/.agentchat/agents/agent_wf_final_reviewer/workdir codex --fresh
# 启动后若 dashboard 列表不显示:重跑一次同样的 up 命令(输出 "Refreshed backend mapping" 即修复注册映射)
```

- [ ] **0.1** `OPENFAB_AGENTCHAT_URL` 已带上(漏了 → console Agents 面板报 "Bridge not configured")
- [ ] **0.2** 公网需求才加 `OPENFAB_ACCESS_TOKEN=$(openssl rand -hex 16)` + Tailscale funnel;纯本地都不要
- [ ] **0.3** openfab 重启后**项目注册表清空**(内存态)——coordinator 会自动恢复,或房里重新 `bind <project>`

## 阶段 0.5:一键体检(30 秒)

```bash
tmux -L agentchat-services ls                     # 应见 5 会话:backend/dashboard/relay/openfab/ofbridge
tmux ls | grep wf_                                # 应见 wf_coordinator / wf_implementer
for p in 8090 8084 8787 8077; do curl -s -o /dev/null -w ":$p → %{http_code}\n" http://127.0.0.1:$p/; done
                                                  # 期望:8090→404、8084→200、8787→200、8077→404(404 是无根路由,正常)
curl -s http://127.0.0.1:8084/api/agents/status | grep -o '"alive":true' | wc -l   # ≥2
cd ~/Work/Projects/FW/openfab && ./target/release/openfab doctor --repo demo/.work/web
```

- [ ] **0.5a** relay 日志有 `Bridge running`,**无** `M_LIMIT_EXCEEDED` 刷屏
- [ ] **0.5b** 桥日志有 `command relay SSE →`;**首次**启动有 `first boot: seeded N…`,以后重启**不得**再出现(出现 = 水位文件被删)
- [ ] **0.5c** dashboard 刚重启后等 3 秒再刷页面(冷启动头 2 秒会假报 "no known agents")。
  过几分钟仍空 → 注册表 tmux 字段脱节,重跑一次 `agent-chat up <name>` 即愈

## 阶段 1:Robrix 建房 + 组队

- [ ] **1.1** 用 trusted-inviter 账号登录(`.env` 的 `MATRIX_TRUSTED_INVITER_MXIDS`;换账号建房会被 enforce 拒——特性)
- [ ] **1.2** 建**带名字**的房
- [ ] **1.3** 邀请 `@ac_wf_coordinator:matrix.palpo.im`(目录可能搜不到,**直接粘贴完整 ID**;不用邀请 bot)
- [ ] **1.4** ≤15s coordinator 自动进房;≤30s bot(@agent-bridge)自动跟进(bot 邀请有轮询兜底,不会永久卡死)。
  失败 → relay 日志搜 `agent-invite` / `bot-invite`;`UNTRUSTED` = 邀请人不对
- [ ] **1.5** **仅当房内 ≥3 个非 bot 成员**才建群(`Created group "<房名>"`)。两人房(你 + 1 agent)走 DM 语义,**没有建群是正常的**,后续流程无差别

## 阶段 2:需求对话(路由验收)

- [ ] **2.1** 房里**不带 @** 发需求 → relay 日志:两人房 `Matrix DM: <你> → wf_coordinator`,群房 `Matrix group: …`
- [ ] **2.2** coordinator 回复**落在同一个房**,日志带 `(reply_thread)` / `(last_room)` 标记
- [ ] **2.3** *(双房并行)*:两个房各发一条 → 各自回复回各自的房(三级线程感知路由:reply_to 源房 → 最近来信房 → 全局 DM 房)
- [ ] **2.4** 对话至 coordinator 产出 requirements + `.spec.md` 并推 OpenFab → dashboard「Incoming from Robrix」出现 spec

> 注意:coordinator 全房共享记忆,新房不是白纸——它可能引用旧项目问你,是特性不是投递错误(会话隔离在待办)。

## 阶段 3:构建(默认 = 机器认证,gate=none)

- [ ] **3.1** 房里 `bind <project>`,再 `build <spec-id>`
- [ ] **3.2** 桥**秒级**回「🛠 Building…」(SSE;>5s = 退化轮询,查桥日志 SSE 断连)
- [ ] **3.3** 身份门:mxid 必须精确映射到唯一 maintainer 才能触发 build;未映射账号发 build 被拒并收到提示
- [ ] **3.4** run 完成:`accepted` + 徽章 **machine**;代码停在 run 分支,**不会自动 merge**(merge 只发生在人工 signoff 路径)
- [ ] **3.5** provenance 诚实性:attestation `generated` = git 实际改动 + 磁盘真实字节哈希(不采信 agent 自报)
- [ ] **3.6** Software 页 git diff 非空

## 阶段 4:可信释放(人工门,可选)

- [ ] **4.1** 桥环境加 `OPENFAB_ROOM_BUILD_GATE=solo` 重启 ofbridge,再 `build`
- [ ] **4.2** run → `blocked / awaiting sign-off`,房里收到 approve 提示
- [ ] **4.3** 房里 `approve <run-id>` → merged,徽章 **human-gate**。
  拒签 → `openfab identity-audit --repo <repo>` 查 mxid 映射(服务器迁移后过期是惯犯)
- [ ] **4.4** `reproduce` 通过(签名 + 源哈希 + agent-spec lifecycle 复验)

## 阶段 5:负向安全(至少抽 3 条)

- [ ] **5.1** 陌生账号建房邀 bot/agent → 拒(enforce + trusted-inviter)
- [ ] **5.2** 陌生账号发 `build x` → 「not authorized」,不烧算力
- [ ] **5.3** 重启 ofbridge → 无历史命令重放(处理水位持久化 + 首启种子)
- [ ] **5.4** Octos 共存:多人房不 @ `@octos_mac` 应沉默(前置:重装 octos-cli 并重启 octos serve 使 mention-gate 生效)
- [ ] **5.5** 任一 agent 无法伪造签核(无 maintainer 凭据必败)

## 已知遗留(不影响验收,按优先级)

1. **launchd 全家桶 + agent keepalive**(所有"死了要人拉"的总根治,含启动依赖顺序)
2. `agent-chat up` 后主动刷新后端注册表 tmux 字段;dashboard 启动先扫一轮 pane 再应答 status
3. openfab 项目注册表落盘(现靠 coordinator 灾后自愈)
4. coordinator 会话隔离(新房 ≠ 白纸);issue-workflow skill 补「回复必带 reply_to」
5. `bind` 时自动 `markRoomTrusted`(下次切 enforce 不再需要手工迁移老房)

---

## 附录:本地 palpo ⇄ 公网 matrix.palpo.im 随时切换

**核心认知:每个 homeserver 是一个独立宇宙**——账号、token、房间 ID、群映射、信任名单、sync 位点全都互不相通。
`@ac_wf_coordinator:127.0.0.1:8128` 和 `@ac_wf_coordinator:matrix.palpo.im` 是**两个无关账号**;桥同一时刻只连一个宇宙,邀请另一个宇宙的账号 = 邀请幽灵(显示"已邀请"但永远没人接)。

状态档案已就位(一次性完成):

```
data/matrix/                          ← 现役状态(relay 读写这里)
data/matrix-profiles/matrix.palpo.im/ ← 公网档案
data/matrix-profiles/local-8128/      ← 本地档案(已种入迁移前的旧 token)
```

### 切换步骤(两个方向对称,约 2 分钟)

```bash
AC=~/Work/Projects/consult/agent-chat; cd "$AC"
TARGET=local-8128          # 或 matrix.palpo.im
CURRENT=matrix.palpo.im    # 与 TARGET 相反

# 1) 停 relay,归档现役状态,换入目标档案
tmux -L agentchat-services kill-session -t relay
rsync -a --delete --exclude 'bridge-owner.lock' data/matrix/ "data/matrix-profiles/$CURRENT/"
rsync -a --delete --exclude 'bridge-owner.lock' "data/matrix-profiles/$TARGET/" data/matrix/

# 2) 改 .env(按目标取一组值)
#    → local-8128:
#      MATRIX_HOMESERVER=http://127.0.0.1:8128
#      MATRIX_SERVER_NAME=127.0.0.1:8128
#      MATRIX_TRUSTED_INVITER_MXIDS=@alex:127.0.0.1:8128
#    → matrix.palpo.im:
#      MATRIX_HOMESERVER=https://matrix.palpo.im
#      MATRIX_SERVER_NAME=matrix.palpo.im
#      MATRIX_TRUSTED_INVITER_MXIDS=@alex:matrix.palpo.im
#    (MATRIX_IGNORED_SENDER_MXIDS 已同时含两个宇宙的 octos,不用改)

# 3) 起 relay
tmux -L agentchat-services new -d -s relay -c "$AC" 'bash -lc "set -a; source .env; set +a; env -u TMUX node bridge-matrix.js >> .demo-logs/relay.log 2>&1"'

# 4) OpenFab 签核身份重绑到目标宇宙的 mxid(否则 approve 被拒)
curl -s -XPOST http://127.0.0.1:8787/api/identity -H 'Content-Type: application/json' \
  -d '{"mxid":"@alex:127.0.0.1:8128","maintainer":"alice"}'   # 切公网时换成 @alex:matrix.palpo.im

# 5) Robrix 登录目标服务器,邀请该宇宙的 agent:@ac_wf_coordinator:<目标 server>
```

### 验证与排错

- [ ] relay 日志 `Bot logged in` + `Bridge running`;若本地旧 token 失效(本地 palpo 数据库重置过)→ 删掉 `data/matrix/bridge-state.json` 里失效的 `agentTokens`/`botToken` 或整个文件后重启,桥会自动重新注册(**缺文件安全;内容为空 `{}` 的文件会崩**)
- [ ] 邀请 ≤15s 被接受(轮询兜底对两个宇宙同样生效)
- [ ] 房间/群/信任名单**不跨宇宙迁移**——目标宇宙里要重新建房/bind/信任(各自档案里各自积累)

### 硬约束

- **本地模式只有本机可达**:`127.0.0.1:8128` 对另一台机器不可见;远程 Robrix 用公网模式,或给本地 palpo 配 LAN IP / Tailscale serve(又回到暴露面权衡)
- 本地 octos 与公网 octos 是两个 bot,忽略名单已双双覆盖
