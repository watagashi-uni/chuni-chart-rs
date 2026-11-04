# 出图与判定规则

README 只介绍用法；这里记录解析、出图和判定图的具体规则。

时间使用小数 BPM 分段积分，保持线性时间轴。分列沿用原项目的小节长度统计与合并规则，列底部对齐，并保留接缝重叠区。SFL / SLP / SLA、旧式 STP 和独立的 DCM 绘制为标记，不改变时间轴；DCM 显示在轨道右侧。CLK 和未知记录忽略，已支持记录的字段与资源限制仍会校验。超界 note 和长条按原坐标计算，只绘制 16 轨以内的部分，完全在轨道外的图形不显示，列宽保持不变。字体栅格化、亚像素坐标和整图缩放可能与浏览器存在少量像素差异。

新变速 `SLP` 的 0 号列表按全局 `SFL` 显示；其他列表由 `SLA` 选择音符范围。`inspect.chart.scroll_profiles` 保存速度程序，`scroll_areas` 独立保存作用时间、轨道范围和列表编号，`scrolls` 是用于静态出图的标记。Rust `Chart::scroll_profile_at` 可查询某个音符位置采用的列表。SLIDE 样式兼容新旧谱面：行尾 `NCL` 关闭中线，保留普通 `SLD` 的轨迹、端点及 EXT 头配色；旧版 `HLD` 使用黄橙色轨迹且无中线，`GRN` 使用绿色轨迹和绿色中线。连接按样式区分，判定类型仍由行首音符关键字决定。省略或未知样式回退为 `SLD`；AIR 颜色字段的 `GRN` 独立处理。格式及 EXE 依据见 [C2S 变速与 SLIDE 样式](c2s-scroll-slide.md)。

判定图提供普通 TAP、HOLD / SLIDE 头和 critical note 的范围及相邻 note 保护边界；FLICK 单独显示触摸入口。AIR、持续判定和实际操作状态不作为 JC / JUSTICE / ATTACK 矩形计算。超界谱支持普通图、判定图和 fastattack：判定与相邻保护只计算音符覆盖的实际 16 个键区，完全在轨道外的装饰音符不生成判定，也不影响轨道内音符的保护。保留原始坐标及 EXT 完整覆盖条件，绘图仍裁切到轨道内。`inspect` 的 `note` 保留原始坐标，`lanes` 按从左到右的实际键区排列，起始键区为 `max(0, note.lane)`。

判定图将普通和 EXT HOLD 头部的延后色带及边缘阴影截到 HOLD 尾部，避免短 HOLD 显示尾后仍可操作的误导；提前色带和相邻音符的保护计算不变，长 HOLD 不会因此延长原有判定范围。此规则只影响出图，`inspect` 和数值判定 API 保留头部内部窗口；行尾 `HLD` 样式的 SLIDE 不按 HOLD 截断。`2147_03` 的 20.833 ms 短 HOLD 及两版 EXE 依据见 [短 HOLD：头部窗口与尾部 MISS](hold-head-and-tail.md)。

保护计算使用静态逆向确认的规则：对所有符合条件的音符对，用**原始时间差的一半**裁剪，并保留 ±2/60 秒的 FIX 下限；不会用另一颗已缩短的窗口重新分配边界。普通 TAP、普通长条起点和 FLICK 触碰阶段将最紧边界传播到整宽（MINIMALIZE）；Critical / EXT 则保留逐键区缺口。两个窗口之间可以留空，JC 内部窗口也可以重叠。

判定图展示的是**内部窗口**。相同档位的底色合并绘制一次，重叠的 JC 不会越叠越亮；每颗窗口用左上亮、右下暗的浅浮雕渐变保留完整形状，包括左右边界和 EXT 阶梯缺口，不再画细线或端点标记。重合处的明暗取最强值，不累加阴影。没有把重叠区强制分配给某一颗音符。实际输入归属取决于音符状态，且 FLICK 等类型的输入路径不同。最终图片中高度不超过 2 像素的 ATTACK 仍使用不透明亮绿色，相关边界不会遮住这段颜色。

**提前 ATTACK 筛选模式**（CLI `--early-attack-only`、HTTP `fastattack=1` 或 `early_attack_only=1`、Rust `Options::early_attack_only`）先用完整谱面计算保护，再选出任一键区仍有正宽度提前 ATTACK 的音符。只绘制这些音符从提前窗口起点到音符时刻的全部 ATTACK / JUSTICE / JC，微小 ATTACK 也会保留；其他判定色带及延后区域隐藏。全部音符本体、长条路径和 AIR 正常显示，包括未选中的音符；原有时间轴、小节、BPM 和节奏标注保留。未选中的音符仍参与保护计算。

同一量化谱面 tick 的 Critical 完整覆盖 TAP、HOLD / SLIDE 头时，目标按 Critical 计算和显示。覆盖可以由多个连续区间拼接，也可以来自已标记 Critical 的长条起点；部分覆盖不升级。解析结果保留原有 `type`，用 `critical: true` 表示有效 EXT 判定。TAP 与 EXT 在游戏的五类判定统计中都归 TAP 类，不能根据统计栏位反推判定类型。

当前只提供默认 HARD / EASY 配置；FLICK 的 TOUCH 外窗口两者均为 ±5/60 秒，不包含 SLASH 的移动条件和最终档位。未复现特殊技能配置、AIR、持续段状态机、逐帧输入竞争或 EXE 浮点舍入的所有细节。

作为 Rust 库使用时，`judgement::protect(&notes, easy)` 按 RESOLUTION=384 处理；其他分辨率请调用 `protect_with_resolution(&notes, easy, resolution)`。`tick` 与 `time` 必须对应同一谱面位置，同刻保护容差为 1/192 个四分音符拍；CLI 和 HTTP 接口自动使用谱面分辨率。
