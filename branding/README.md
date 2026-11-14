# voloo 品牌字标

产品名使用小写 **voloo**。

- `voloo-wordmark.svg`：独立产品字标，透明底、墨紫色。
- `librezoo-voloo.svg`：完整 librezoo 图文标与 voloo 的横排组合，用于海报页头。
- `librezoo-voloo-small.svg`：小尺寸横排组合，采用 LibreZoo 母版的小尺寸圆点配色。
- `librezoo-voloo-stacked.svg`：窄屏组合，保持完整 librezoo 名称与 voloo 字标。
- `*-on-dark.svg`：对应的深底版本。
- `png/`：以上各版本的透明底 PNG，可直接用于排版。
- `preview.html`：字标、母品牌组合及海报页头的预览。

这些 SVG 已将字形矢量化，使用时不需要安装字体。`l` 和 `o` 直接沿用 LibreZoo 定制小写字标的原轮廓；`v` 按同一字高、笔画及圆角风格补绘。LibreZoo 图文组合保持原母版，不重排其字形或移动圆点。

运行 `python3 branding/build.py` 可从 `source/` 重新生成。母版来源为 [librezoo/branding](https://github.com/librezoo/branding)，使用时保留留白，不拉伸、不加描边或阴影。

品牌标识用于识别 LibreZoo 与 voloo，不随软件的 Apache-2.0 许可证授权。

宣传页头采用左侧完整品牌组合、右侧 `Agent-native Hardware` 与「电源开关」。
