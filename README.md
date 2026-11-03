# chuni-chart-rs

C2S 谱面预览服务。Rust 直接生成 PNG / JPEG，沿用 [MarisaBot / Marisa.Frontend](https://github.com/QingQiz/MarisaBot) 的灰色轨道、小节分列、圆角 note、slide 渐变和 AIR 样式。

部署者把谱面文件放进目录，Bot 或其他 HTTP 客户端通过文件名请求图片。服务只提供 API，不提供网页。运行时只需要一个二进制文件，字体已内嵌。

![自制示例谱面](assets/example.png)

## 快速启动与配置

从 [Releases](https://github.com/watagashi-uni/chuni-chart-rs/releases) 下载 Linux x64 静态包或 Windows x64 ZIP。Windows 包内的 `.exe` 使用静态 C 运行库。每个版本提供 `SHA256SUMS`，安装包不包含真实谱面或真实配置。

Linux：

```sh
./chuni-chart-rs init-config
# 编辑 config.json 的 chart_dir，保留生成的 etag_salt
./chuni-chart-rs serve --config config.json
```

Windows PowerShell：

```powershell
.\chuni-chart-rs.exe init-config
notepad config.json
.\chuni-chart-rs.exe serve --config config.json
```

`init-config` 生成含随机盐的配置，拒绝覆盖已有文件；Linux 下权限为 `0600`。也可以复制 `config.example.json` 后编辑：

```json
{
  "chart_dir": "charts",
  "listen": "127.0.0.1:3000",
  "timeout_seconds": 15,
  "render_workers": 4,
  "queue_capacity": 32,
  "queue_timeout_seconds": 30,
  "etag_salt": null
}
```

默认同时出 4 张图，再允许 32 个请求排队，排队最多 30 秒。4 核 4 GB 的 Windows 可以先用这组配置；机器还运行其他较重服务时，将 `render_workers` 调成 2。旧配置未填写新增字段时自动采用这些默认值。

| 配置 | 含义 | 可填范围 |
| --- | --- | --- |
| `render_workers` | 同时出图的数量，每张图由独立进程处理 | 1–16，默认 4 |
| `queue_capacity` | 出图忙时最多再等多少个请求 | 0–128，默认 32；0 表示不排队 |
| `queue_timeout_seconds` | 一个请求最多排队多少秒 | 1–120，默认 30 |
| `timeout_seconds` | 开始出图后的处理超时 | 1–30，默认 15 |

排队等待使用异步任务，不占一个专门的等待线程，也不提前加载整份谱面。客户端和反向代理的超时需要大于排队时间加出图时间，默认配置可设为 60 秒。增大并发前，也要增加部署环境的总内存预算。

`chart_dir` 必须直接包含 `1086_03.c2s` 等文件，不递归寻找子目录。配置中的相对路径相对于配置文件所在目录；Windows 绝对路径可写为 `D:/charts/c2s`。`listen` 使用 IP 和端口，默认仅监听本机；`timeout_seconds` 为 1–30 秒。

建议保留 `init-config` 生成的 `etag_salt`，让 ETag 跨重启稳定。`null` 会在每次启动时生成临时随机盐，旧 ETag 因而失效。手动设置的盐需要 32–4096 字节；不要提交真实配置，或把盐发给客户端。

`serve` 默认读取当前目录的 `config.json`；未找到时使用默认设置。显式指定的配置文件不存在或格式错误时拒绝启动。`--config` / `CHUNI_CHART_CONFIG` 指定配置位置；`--chart-dir` / `CHART_DIR`、`--listen` / `LISTEN` 和 `--timeout` 覆盖配置中的对应值，显式命令行参数优先于环境变量。`ETAG_SALT` 可覆盖配置中的盐。

从源码构建：

```sh
cargo build --release --locked
./target/release/chuni-chart-rs init-config
mkdir -p charts
# 放入 C2S 文件并编辑 config.json
./target/release/chuni-chart-rs serve
```

启动后访问 `http://127.0.0.1:3000/healthz`，返回 `ok` 即表示服务已启动。图片通过下面的 API 获取；根路径 `/` 返回 404。Windows 出图子进程以无控制台窗口方式启动，不会为每次出图创建控制台窗口。

## 图片接口

```text
GET /preview?name=1086_03
GET /api/render/1086_03
GET /api/render?name=1086_03&format=jpg
GET /judge?name=1086_03
GET /fastattack/1086_03
GET /api/render/1086_03?fastattack=1
GET /judge?name=2140_03&early_attack_only=1
GET /api/render?name=1086_03&judge=1&format=png
GET /preview?name=1086_03&column=3
GET /healthz
```

成功直接返回图片字节，可供 Bot 下载发送。`name` 可以带 `.c2s` 后缀，文件名仅支持字母、数字、下划线和连字符，长度不超过 80；不接受目录路径。

| 参数 | 默认值 | 用途 |
| --- | --- | --- |
| `name` | 查询接口必填 | 谱面文件名；也可使用 `/api/render/1086_03` 路径形式，不要重复传入 |
| `format` | `png` | `png`、`jpg` 或 `jpeg` |
| `judge` | `0` | `1` 显示判定范围；`/judge` 固定开启 |
| `fastattack` | `0` | `1` 保留全部 note，只画仍有提前 ATTACK 的音符的提前 ATTACK / JUSTICE / JC；自动开启判定模式 |
| `early_attack_only` | `0` | `fastattack` 的兼容别名 |
| `div` | `1` | `0` 隐藏音符右侧的分度值（如 `1/16`、`1/24`），`1` 显示；兼容 Chunibot `newchart` 参数 |
| `easy` | `0` | `1` 使用 BASIC / ADVANCED 时间窗口 |
| `column` | 全谱 | 从 1 开始的列号，便于查看局部细节 |
| `zoom` | `1` | 纵向比例，范围 0.5–2 |

找不到谱面返回 `404`；输入过大返回 `400`；解析失败、资源超限或超时返回 `422`；出图忙时请求先排队等待，只有队列已满或排队超时才返回 `429` 和 `Retry-After: 2`。调用方可稍后重试。非法参数返回 `400`，超长 URI / 条件请求头返回 `414` / `431`。服务不预热、不保存服务器端图片缓存。

关闭音符间隔的分度值显示，在对应接口加 `div=0` 即可，三种图片模式都支持：

```text
GET /api/render/1086_03?div=0
GET /judge?name=1086_03&div=0
GET /fastattack/1086_03?div=0
```

命令行出图使用 `--no-divisions`。显示和隐藏分度值分别使用各自的 ETag，客户端缓存不会混用。

## ETag 与条件请求

成功返回图片时附带 `ETag: W/"…"` 和 `Cache-Control: no-cache`。客户端保存图片和这个不透明值，下次请求**相同图片参数**时原样发送 `If-None-Match`；谱面和渲染条件未变时返回 `304 Not Modified`，无图片正文，也不启动渲染子进程。

```http
GET /api/render/1086_03?fastattack=1 HTTP/1.1
If-None-Match: W/"上次响应中的完整摘要"
```

ETag 使用 HMAC-SHA-256，包含谱面内容、标准化后的模式 / 难度 / 格式 / 缩放 / 列号 / 间隔标注，以及渲染代码、字体、依赖锁定文件和目标平台构成的构建指纹。它不返回原始谱面 SHA-256、文件路径或盐。只修改文件时间不会失效；内容、模式、构建或盐变化会失效。支持弱比较、多个候选标签和 `*`；通配条件先确认该谱面能成功渲染，不会替无效谱面返回 304。

Chunibot 的 `newchart` / `chujudge` 可将 `CHUNIBOT_CHART_PNG_URL` 指向本服务的 `/api/render`，并启用 `CHUNIBOT_CHART_ETAG_ENABLED=1`。其 `name`、`judge`、`div` 和 `If-None-Match` 参数均可直接使用。本项目不会修改 Chunibot 配置或缓存。

## 访问日志

启动服务后默认向控制台的标准错误输出访问日志，每个 HTTP 请求一行。例如：

```text
2026-11-03T12:00:00+08:00: [render] "GET /api/render/1086_03?div=0 HTTP/1.1" 200 1412835 830.5ms
2026-11-03T12:00:02+08:00: [render] "GET /api/render/1086_03?div=0 HTTP/1.1" 304 0 0.6ms
```

依次是服务器本地时间和时区、请求方法及地址、HTTP 版本、状态码、生成的响应正文大小（字节）和耗时。耗时包含排队、读取和渲染，不包含客户端下载图片的时间。成功、304 缓存命中及失败请求都会记录；检查服务的 `/healthz` 请求也会记录。

日志不记录请求头、配置盐或谱面正文；未知查询参数会显示为 `[redacted]`，长字段会截断，控制字符会转义。日志在独立线程写入，最多缓冲 512 行，输出长期阻塞时丢弃超出的记录并在恢复后汇总数量，避免影响 API 请求。日志可直接由运行服务的终端或服务管理器收集。

## 命令行出图

```sh
./target/release/chuni-chart-rs render charts/1086_03.c2s -o chart.png
./target/release/chuni-chart-rs render charts/1086_03.c2s --column 3 -o detail.png
./target/release/chuni-chart-rs render charts/1086_03.c2s --judge -o judge.jpg
./target/release/chuni-chart-rs render charts/2140_03.c2s --early-attack-only -o early-attack.png
./target/release/chuni-chart-rs inspect charts/1086_03.c2s > timing.json
```

时间使用小数 BPM 分段积分，保持线性时间轴。分列沿用原项目的小节长度统计与合并规则，列底部对齐，并保留接缝重叠区。SFL / SV2 绘制为标记，不改变时间轴。超界 note 和长条按原坐标计算，只绘制 16 轨以内的部分，完全在轨道外的图形不显示，列宽保持不变。字体栅格化、亚像素坐标和整图缩放可能与浏览器存在少量像素差异。

新变速 `SLP` 的 0 号列表按全局 `SFL` 显示；其他列表由 `SLA` 选择音符范围。`inspect.chart.scroll_profiles` 保存速度程序，`scroll_areas` 独立保存作用时间、轨道范围和列表编号，`scrolls` 是用于静态出图的标记。Rust `Chart::scroll_profile_at` 可查询某个音符位置采用的列表。SLIDE 样式兼容新旧谱面：行尾 `NCL` 关闭中线，保留普通 `SLD` 的轨迹、端点及 EXT 头配色；旧版 `HLD` 使用黄橙色轨迹且无中线，`GRN` 使用绿色轨迹和绿色中线。连接按样式区分，判定类型仍由行首音符关键字决定。省略或未知样式回退为 `SLD`；AIR 颜色字段的 `GRN` 独立处理。格式及 EXE 依据见 [C2S 变速与 SLIDE 样式](docs/c2s-scroll-slide.md)。

判定图提供普通 TAP、HOLD / SLIDE 头和 critical note 的范围及相邻 note 保护边界；FLICK 单独显示触摸入口。AIR、持续判定和实际操作状态不作为 JC / JUSTICE / ATTACK 矩形计算。超界谱支持普通图、判定图和 fastattack：判定与相邻保护只计算音符覆盖的实际 16 个键区，完全在轨道外的装饰音符不生成判定，也不影响轨道内音符的保护。保留原始坐标及 EXT 完整覆盖条件，绘图仍裁切到轨道内。`inspect` 的 `note` 保留原始坐标，`lanes` 按从左到右的实际键区排列，起始键区为 `max(0, note.lane)`。

判定图将普通和 EXT HOLD 头部的延后色带及边缘阴影截到 HOLD 尾部，避免短 HOLD 显示尾后仍可操作的误导；提前色带和相邻音符的保护计算不变，长 HOLD 不会因此延长原有判定范围。此规则只影响出图，`inspect` 和数值判定 API 保留头部内部窗口；行尾 `HLD` 样式的 SLIDE 不按 HOLD 截断。`2147_03` 的 20.833 ms 短 HOLD 及两版 EXE 依据见 [短 HOLD：头部窗口与尾部 MISS](docs/hold-head-and-tail.md)。

保护计算使用静态逆向确认的规则：对所有符合条件的音符对，用**原始时间差的一半**裁剪，并保留 ±2/60 秒的 FIX 下限；不会用另一颗已缩短的窗口重新分配边界。普通 TAP、普通长条起点和 FLICK 触碰阶段将最紧边界传播到整宽（MINIMALIZE）；Critical / EXT 则保留逐键区缺口。两个窗口之间可以留空，JC 内部窗口也可以重叠。

判定图展示的是**内部窗口**。相同档位的底色合并绘制一次，重叠的 JC 不会越叠越亮；每颗窗口用左上亮、右下暗的浅浮雕渐变保留完整形状，包括左右边界和 EXT 阶梯缺口，不再画细线或端点标记。重合处的明暗取最强值，不累加阴影。没有把重叠区强制分配给某一颗音符。实际输入归属取决于音符状态，且 FLICK 等类型的输入路径不同。最终图片中高度不超过 2 像素的 ATTACK 仍使用不透明亮绿色，相关边界不会遮住这段颜色。

**提前 ATTACK 筛选模式**（CLI `--early-attack-only`、HTTP `fastattack=1` 或 `early_attack_only=1`、Rust `Options::early_attack_only`）先用完整谱面计算保护，再选出任一键区仍有正宽度提前 ATTACK 的音符。只绘制这些音符从提前窗口起点到音符时刻的全部 ATTACK / JUSTICE / JC，微小 ATTACK 也会保留；其他判定色带及延后区域隐藏。全部音符本体、长条路径和 AIR 正常显示，包括未选中的音符；原有时间轴、小节、BPM 和节奏标注保留。未选中的音符仍参与保护计算。

同一量化谱面 tick 的 Critical 完整覆盖 TAP、HOLD / SLIDE 头时，目标按 Critical 计算和显示。覆盖可以由多个连续区间拼接，也可以来自已标记 Critical 的长条起点；部分覆盖不升级。解析结果保留原有 `type`，用 `critical: true` 表示有效 EXT 判定。TAP 与 EXT 在游戏的五类判定统计中都归 TAP 类，不能根据统计栏位反推判定类型。

当前只提供默认 HARD / EASY 配置；FLICK 的 TOUCH 外窗口两者均为 ±5/60 秒，不包含 SLASH 的移动条件和最终档位。未复现特殊技能配置、AIR、持续段状态机、逐帧输入竞争或 EXE 浮点舍入的所有细节。

作为 Rust 库使用时，`judgement::protect(&notes, easy)` 按 RESOLUTION=384 处理；其他分辨率请调用 `protect_with_resolution(&notes, easy, resolution)`。`tick` 与 `time` 必须对应同一谱面位置，同刻保护容差为 1/192 个四分音符拍；CLI 和 HTTP 接口自动使用谱面分辨率。

## Linux 部署

建议使用 systemd 管理二进制服务。仓库附带 [service 文件](deploy/chuni-chart.service)，默认仅监听本机，外部访问由部署者的反向代理转发。

```sh
git clone https://github.com/watagashi-uni/chuni-chart-rs.git
cd chuni-chart-rs
cargo build --release --locked
sudo useradd --system --no-create-home --shell /usr/sbin/nologin chuni-chart
sudo install -d /opt/chuni-chart/bin /srv/chuni-chart/charts
sudo install -d -o chuni-chart -g chuni-chart /etc/chuni-chart
sudo install -m 755 target/release/chuni-chart-rs /opt/chuni-chart/bin/
sudo install -m 644 deploy/chuni-chart.service /etc/systemd/system/
sudo -u chuni-chart /opt/chuni-chart/bin/chuni-chart-rs init-config -o /etc/chuni-chart/config.json
# 编辑 /etc/chuni-chart/config.json，将 chart_dir 设为 /srv/chuni-chart/charts
# 将谱面放入 /srv/chuni-chart/charts，并让 chuni-chart 用户可以读取
sudo systemctl daemon-reload
sudo systemctl enable --now chuni-chart
```

更新时在源码目录 `git pull --ff-only`，重新编译并安装二进制，然后 `sudo systemctl restart chuni-chart`。预编译 Linux musl 包可以从 Releases 下载。

资源限制：

- 默认同时运行 4 个渲染子进程，最多再等待 32 个请求。并发、队列容量和排队超时可配置；排队请求不保存谱面正文或图片。
- 输入上限 20 MiB，最多 12,000 个 note / 长条 / 变速事件，谱面最长 600 秒。
- 整图优先保留原始像素尺寸和字体大小；超过预算时才等比缩小至最多 4,000 万像素，单边最多 32,768 像素，编码结果最多 16 MiB。
- Linux 子进程地址空间上限 256 MiB、CPU 时间上限 12 秒；Windows 使用 Job Object 限制进程提交内存 256 MiB、用户 CPU 时间 12 秒，并禁止再创建子进程。默认墙钟超时 15 秒，超时终止并回收。
- systemd 示例按默认 4 并发配置，限制整个服务及其子进程最多 1.5 GiB，禁用 swap；提高并发时应相应调整总内存上限。Windows 使用独立的 Job Object；其他系统仍有输入、像素、并发及超时限制。

HTTP 只提供图片 API 和 `/healthz` 健康检查，不提供网页、谱面下载、目录浏览、配置读取或 `inspect` API。谱面根目录通过句柄约束，拒绝符号链接 / 重解析点及特殊文件；错误响应不返回源文件内容。详细边界、检查项目和剩余依赖维护提示见 [SECURITY.md](SECURITY.md)。

## 开发验证

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
python3 scripts/smoke.py
python3 scripts/security_http.py
python3 scripts/queue_http.py
python3 scripts/access_log.py
cargo audit --deny unsound
```

测试覆盖小数 BPM、长条连接、EXE 规则的明确数值案例、120 组随机输入的边界与顺序不变性、JC 重叠填色不加深、提前 ATTACK 筛选与提前三档保留、微小 ATTACK 显示、文件名边界、PNG/JPEG 出图，以及真实 HTTP 子进程的模式传递、并行出图、排队、队列满及等待超时、超时回收和失败恢复。测试谱面为自制样例。安全回归另外覆盖内容变化 / 参数变化 / 跨重启的 ETag、304 绕过渲染、路径穿越、符号链接与文件替换竞态、特殊文件、错误信息隔离和请求大小限制。Windows 原生测试额外验证出图进程的启动设置不会创建控制台窗口。

可选：与旧 JavaScript 实现比较自己的 C2S 文件。脚本校验解析结果，并报告预期存在的判定差异；旧判定结果不再作为正确性基准：

```sh
node scripts/compare-reference.cjs /path/to/chuni-judgement /path/to/chart.c2s
```

## License

AGPL-3.0-only。原项目和数值规则参考见 [NOTICE](NOTICE)，字体许可见 [assets/FONT-LICENSE](assets/FONT-LICENSE)。
