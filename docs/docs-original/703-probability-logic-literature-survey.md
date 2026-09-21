# #703 確率ロジック刷新のための文献調査所見（2026-09-21）

Web 上の学術・実務文献の調査所見（一次資料）。調査は 4 トピック並列（確率校正・Kelly/資金管理・
Harville/順位確率合成・市場効率性）＋統括の計 5 レポートで実施した。ここには**確定知に蒸留する
候補となった所見**だけを、出典 URL 付きで残す。蒸留先: `docs/specifications/backtest.md`
（評価プロトコル）・`docs/specifications/probability-estimation.md`（Phase 2/3 で追記予定）。

## 1. ブレンド結合形（Phase 3 の根拠）

- **Ranjan & Gneiting (2010), JRSS-B 72(1)**: 校正済み予測同士の線形プールは必然的に非校正かつ
  underconfident（定理）。重み最適化でも消えない。対処は混合後の再校正 or 結合形の変更。
  <https://academic.oup.com/jrsssb/article/72/1/71/7076442>
- **Benter (1994)**: `c_i ∝ exp(α·ln f_i + β·ln π_i)`＝対数プール。α,β は条件付きロジット MLE。
  評価は擬似 R²（Bolton-Chapman）の**対市場増分 ΔR²**で、実運用モデル ΔR²=+0.0178・無価値な
  tipster モデル +0.0002。「モデルと公衆が食い違うとき実際は公衆側に寄る」→ 生モデル確率での
  EV 値付けを明示的に警告。<https://gwern.net/doc/statistics/decision/1994-benter.pdf>
- **Sung & Johnson (2007), J. Prediction Markets 1(1)**: オッズを特徴量として 1 段で入れるより
  Benter 式 2 段階の方が利益が実質的に大きい（英国データ）。
  <https://ideas.repec.org/a/buc/jpredm/v1y2007i1p43-59.html>
- **留保**: ADR 0053 内の実験（learned-model-harness.md:834-844）は「PL に log_market を生特徴量と
  並置すると β_market≈1.04・fundamental 係数崩壊」を観測済み。#703 Phase 3 は合成済み p_model を
  1 入力とするレース単位条件付き尤度＋fit/eval 分離で、この棄却時の検証条件を上回る測定を先に行う。
- **慶應・長倉研 (2020)**: 2018 年 JRA 全レースで単勝支持率≒実勝率（市場は高効率）→ 対数プール化の
  改善幅は小さい可能性。純モデル確率での +EV フィルタは回収率を悪化させ（91%→67%）、確率下限の
  併用で反転（複勝 104〜109%）。<https://user.keio.ac.jp/~nagakura/mitaronsotsuron/mitaron2020b.pdf>

## 2. 順位確率合成（Phase 2 の根拠）

- **Harville (1973), JASA 68**: `π_ijk = π_i · π_j/(1−π_i) · π_k/(1−π_i−π_j)`。指数（Gumbel）走破
  タイムの仮定と同値。<https://www.tandfonline.com/doi/abs/10.1080/01621459.1973.10482425>
- **Benter (1994) Table 9/10（香港 3,198R）**: 素の Harville は条件付き 2 着確率で高確率帯 Z=−4.3、
  低確率帯 Z=+5.3、3 着は Z=−8.3——「強い馬の 2・3 着を過大評価、弱い馬を過小評価」の一次証拠。
  補正形: `σ_i ∝ π_i^γ`（2 着用）/ `τ_i ∝ π_i^δ`（3 着用）を Harville 形に代入。香港 MLE 値
  γ=0.81 / δ=0.65 で Z がほぼ消える。**Note 3**: 市場由来の勝率（FLB あり）では Harville バイアスと
  部分相殺するため、pure と blended で最適 λ は別物。
- **Lo & Bacon-Shone (2008), Handbook of Sports and Lottery Markets**: 上記 discount 形は
  Henery（正規）/ Stern（ガンマ）の重い数値積分を Harville 同等の計算量で近似する。対数尤度が
  着順位置ごとに分離するため λ2/λ3 は独立の 1 次元 MLE で推定できる。
  <https://www.sciencedirect.com/science/article/abs/pii/B978044450744050007X>
- **伊藤耕介 (2010), 大阪商業大学アミューズメント産業研究所紀要 12**: **JRA 44,774R**（1995〜2007）で
  λ2=0.81 / λ3=0.70（Stern r=40 相当）が Harville より対数尤度 1,500 優る（Harville −153,455.5 →
  r=40 −151,952.9。Henery r=∞ は −152,030.6 でわずかに劣る）。複勝の周辺確率式・JRA 規則（8 頭以上
  3 着内 / 5〜7 頭 2 着内）の分岐も明示。EV 実測では「全馬対象だと E>1.0 帯の回収率が逆に悪化
  （65.6%）・単勝支持率 ≥20% に限ると 0.8<E≤1.0 で 102.06%」——大穴帯は支持率自体が勝率を過大評価
  しており EV の入口が汚染されるため。<https://ssrs.dpri.kyoto-u.ac.jp/itokosk/download/paper/Ito2010_Keiba.pdf>
- **Ali (1998), J. Applied Statistics 25(2)**: 15,402R で最良の正規順位モデルでも残差バイアスは
  消えない（1 番人気 2 着 Z=+5.74）。「どの順位モデルでも収益戦略は組めない」——λ 補正は確率合成の
  正しさを直すだけでエッジは生まない（軸ロック＋ズレ増額がエッジという現行整理と整合）。
  <https://www.stat.berkeley.edu/~aldous/157/Papers/ali.pdf>

## 3. 評価・計測（Phase 1 の根拠）

- **Dimitriadis, Gneiting & Jordan (2021), PNAS（CORP）**: PAV（isotonic）でビン境界をデータから
  最適決定した reliability diagram。リサンプリングで consistency band が引け、数千 R 規模でも
  「ズレが有意か」を判定できる。スコア分解 S̄ = MCB − DSC + UNC が付随。
  <https://www.pnas.org/doi/10.1073/pnas.2016191118>（Triptych: <https://arxiv.org/pdf/2301.10803>）
- **Roelofs et al. (2022), AISTATS / Vaicenavicius et al. (2019), AISTATS**: ECE はどのビニングでも
  バイアスを持ち、絶対値を目標にできない。**Ferro & Fricker (2012), QJRMS**: 素の Brier 3 分解は
  小標本で reliability を過大評価（#703 では PAV を共有できる CORP 分解を正とし、FF 補正 3 分解は
  採らなかった——ビン選択の恣意性ごと消えるため）。
- **CalArena (2026, arXiv:2605.30188)**: 大規模比較で「少パラメータの平滑なロジスティック族が優位・
  isotonic は不連続が害・Venn-Abers はほぼ効かない・評価は ECE でなく Brier の事後改善量」。
  #319 の isotonic 棄却と整合。

## 4. 市場効率性（follow-up issue の根拠・#703 スコープ外）

- **Hanyu, Ishii, Otani & Teramoto (2025/2026, arXiv:2509.14645)**: JRA 2004〜2023・約 63,000R・
  5 分刻みオッズ。**最終オッズは十分統計量ではない**——同一最終オッズなら直前 5 分でオッズが
  **下がった**馬の実現回収率が有意に高い（δ=−0.3559）。全投票の約半数が締切 5 分前以降。
  JRA 単勝の FLB は「存在するが控除率を覆さない」（本命帯 0.9〜1.0 / 100 倍超 0.7〜0.8）。
  → 現行「ズレ増額」（オッズが上がって割安に見えたら増額）と**符号が逆**の可能性。要実データ検証。
- **小幡・太宰 (2014), 行動経済学 7**: 2009 年 JRA 全 3,453R・三連単全組合せの実得票数。三連単の
  本命サイドは単勝から Harville 合成した理論確率より票が薄い（人気度 0.822）、大穴サイドは 157 倍の
  過剰人気。ただし理論確率が Harville なので本命側の値は補正なしには割安ともバイアスとも言えない。
  <https://www.jstage.jst.go.jp/article/jbef/7/0/7_1/_article/-char/ja/>
- **芦谷政浩 (2010), 国民経済雑誌 202(2)**: JRA 2008/2009 全レースの実測で複勝本命帯の払戻率 ≥0.85。
  部分標本で +EV に見えるフィルタが合算で死ぬ実例（良馬場 1.3 倍複勝 103.6%→合算 99.3%）。
  <https://da.lib.kobe-u.ac.jp/da/kernel/81006950/81006950.pdf>
- **Walls & Busche (2003), Applied Economics Letters 10(5)**: 香港・日本の実得票額データでは FLB が
  検出されず、丸めオッズでのみ検出——日本市場に米国流 FLB 補正を無条件に当てるのは危険。
  <https://www.tandfonline.com/doi/abs/10.1080/13504850210147162>

## 5. Kelly・資金管理（参考・当面適用なし）

- **Whelan (2025), Bulletin of Economic Research**: 排反アウトカムの同時最適賭けは per-bet Kelly より
  aggressive になるが、「オッズにマージンが乗りかつ真の確率を反映しているなら aggressive な解の方が
  損失が大きい」——JRA の控除率 20〜25% はこの但し書きが直撃。ADR 0046（確率重み配分の棄却）と
  独立に整合する理論的根拠。<https://www.karlwhelan.com/Papers/BER.pdf>
- **離散化の下限**: 券種予算 ¥1,500 ÷ ¥100 = 15 ユニット・相手 5 頭で 1 脚 3 ユニット。Kelly/確率
  重みの微細な重み差はこの粒度でほぼ丸め消える——ADR 0046 の棄却は「理論が誤り」ではなく「¥5,000 の
  粒度で表現できる解像度を超えていた」可能性が高い（配分ルール変更を再提案する場合の測定設計上の注意）。
- **Smith & Winkler (2006), Management Science（Optimizer's Curse）**: 推定 EV が不偏でも EV 順選択は
  選ばれた案の実現値を系統的に下振れさせる。ADR 0076 の Spearman≈0 の説明要素。
  <https://jimsmith.host.dartmouth.edu/wp-content/uploads/2022/04/The_Optimizers_Curse.pdf>
