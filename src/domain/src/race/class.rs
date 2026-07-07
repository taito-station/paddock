use crate::error::Error;

/// レースの格付け／条件クラス（#345）。重賞（G1/G2/G3）から条件戦・新馬までを
/// **順序尺度**（G1 が最上位、新馬が最下位）として表す。netkeiba のレース名・
/// 条件表記から `from_label` で正規化し、DB へは `as_str` の安定スラッグで保存する。
///
/// 重賞判定 `is_graded` は JRA の定義に合わせ **G1/G2/G3 のみ**（リステッドは重賞外）。
/// G1 裏レース検出（別場の非重賞）で「非重賞」＝`!is_graded()` として使う。
///
/// 変種は **下位→上位の昇順**で宣言し、derive した `Ord` が「新馬 < … < G1」の
/// 順序尺度になるようにする（G1 が最大＝最上位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RaceClass {
    /// 新馬
    NewComer,
    /// 未勝利
    Maiden,
    /// 1勝クラス（旧 500万下）
    Win1,
    /// 2勝クラス（旧 1000万下）
    Win2,
    /// 3勝クラス（旧 1600万下）
    Win3,
    /// オープン特別（非リステッドの OP）
    Open,
    /// リステッド（L）
    Listed,
    /// G3
    G3,
    /// G2
    G2,
    /// G1
    G1,
}

impl RaceClass {
    /// DB 保存用の安定スラッグ。`TryFrom<&str>` と対で往復する。
    pub fn as_str(&self) -> &'static str {
        match self {
            RaceClass::G1 => "g1",
            RaceClass::G2 => "g2",
            RaceClass::G3 => "g3",
            RaceClass::Listed => "listed",
            RaceClass::Open => "open",
            RaceClass::Win3 => "win3",
            RaceClass::Win2 => "win2",
            RaceClass::Win1 => "win1",
            RaceClass::Maiden => "maiden",
            RaceClass::NewComer => "newcomer",
        }
    }

    /// 重賞（G1/G2/G3）か。リステッドは JRA 定義で重賞外なので false。
    pub fn is_graded(&self) -> bool {
        matches!(self, RaceClass::G1 | RaceClass::G2 | RaceClass::G3)
    }

    /// G1 か。
    pub fn is_g1(&self) -> bool {
        matches!(self, RaceClass::G1)
    }

    /// netkeiba のレース名／条件表記からクラスを正規化する。
    ///
    /// グレード表記（(G1)/(GI)、(G2)/(GII)、(G3)/(GIII)）を最優先で見て、
    /// 無ければリステッド → 新馬 → 未勝利 → n勝クラス → オープン の順に条件語を拾う。
    /// カード側では `<title>`（グレードを含む）と `RaceData02`（条件を含む）を結合した
    /// ラベルを渡す想定。どれにも当てはまらなければ `None`。
    ///
    /// - グレードは算用数字(G1)とローマ数字(GI)の両表記に対応し、**括弧付き表記にアンカー**する
    ///   （GIII→GII→GI の順で最長一致）。レース名・スポンサー名中に偶発的に混じる裸の "GI"/"G1"
    ///   部分文字列を誤検出しないため、必ず `(GI)`/`(G1)` の括弧付きで判定する。
    /// - 地方交流重賞の Jpn1/Jpn2/Jpn3 表記と unicode ローマ数字（GⅠ 等）・全角括弧は非対応で
    ///   `None` に落ちる（JRA 中央スコープの best-effort。必要になれば拡張する）。
    pub fn from_label(label: &str) -> Option<Self> {
        // netkeiba の RaceData02 は数字を全角でレンダする（出馬表 fixture の「サラ系３歳以上」も
        // 全角３）。n勝クラス（１勝/２勝/３勝）を取りこぼさないよう、判定前に全角数字だけ半角へ
        // 寄せる。グレードは title 側が半角のため無影響。かな等は触らない（focused な正規化）。
        let label: String = label
            .chars()
            .map(|c| match c {
                '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
                other => other,
            })
            .collect();
        // 1. グレード（括弧付きにアンカー・最長一致: (GIII)/(GII)/(GI)）
        if label.contains("(GIII)") || label.contains("(G3)") {
            return Some(RaceClass::G3);
        }
        if label.contains("(GII)") || label.contains("(G2)") {
            return Some(RaceClass::G2);
        }
        if label.contains("(GI)") || label.contains("(G1)") {
            return Some(RaceClass::G1);
        }
        // 2. リステッド
        if label.contains("(L)") || label.contains("（L）") || label.contains("リステッド") {
            return Some(RaceClass::Listed);
        }
        // 3. 条件（新馬・未勝利が「n勝」より先。文字列の包含衝突を避ける）
        if label.contains("新馬") {
            return Some(RaceClass::NewComer);
        }
        if label.contains("未勝利") {
            return Some(RaceClass::Maiden);
        }
        if label.contains("3勝") {
            return Some(RaceClass::Win3);
        }
        if label.contains("2勝") {
            return Some(RaceClass::Win2);
        }
        if label.contains("1勝") {
            return Some(RaceClass::Win1);
        }
        // 4. オープン特別（グレード・リステッドで拾えなかった OP）
        if label.contains("オープン") || label.contains("(OP)") || label.contains("ＯＰ") {
            return Some(RaceClass::Open);
        }
        None
    }
}

/// 画面・通知表示用の日本語短縮表記。
impl std::fmt::Display for RaceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RaceClass::G1 => "G1",
            RaceClass::G2 => "G2",
            RaceClass::G3 => "G3",
            RaceClass::Listed => "L",
            RaceClass::Open => "OP",
            RaceClass::Win3 => "3勝",
            RaceClass::Win2 => "2勝",
            RaceClass::Win1 => "1勝",
            RaceClass::Maiden => "未勝利",
            RaceClass::NewComer => "新馬",
        };
        f.write_str(s)
    }
}

/// DB スラッグ → RaceClass。`as_str` の逆変換。
impl TryFrom<&str> for RaceClass {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "g1" => Ok(RaceClass::G1),
            "g2" => Ok(RaceClass::G2),
            "g3" => Ok(RaceClass::G3),
            "listed" => Ok(RaceClass::Listed),
            "open" => Ok(RaceClass::Open),
            "win3" => Ok(RaceClass::Win3),
            "win2" => Ok(RaceClass::Win2),
            "win1" => Ok(RaceClass::Win1),
            "maiden" => Ok(RaceClass::Maiden),
            "newcomer" => Ok(RaceClass::NewComer),
            other => Err(Error::InvalidFormat(format!("unknown race class: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_label_reads_grade_both_notations() {
        assert_eq!(RaceClass::from_label("安田記念(G1)"), Some(RaceClass::G1));
        assert_eq!(RaceClass::from_label("有馬記念(GI)"), Some(RaceClass::G1));
        assert_eq!(RaceClass::from_label("札幌記念(GII)"), Some(RaceClass::G2));
        assert_eq!(RaceClass::from_label("毎日杯(GIII)"), Some(RaceClass::G3));
        // 算用数字 (G2)/(G3) 直接ケース（最長一致が算用側でも効く）。
        assert_eq!(RaceClass::from_label("レース(G2)"), Some(RaceClass::G2));
        assert_eq!(RaceClass::from_label("レース(G3)"), Some(RaceClass::G3));
    }

    #[test]
    fn is_g1_only_true_for_g1() {
        assert!(RaceClass::G1.is_g1());
        assert!(!RaceClass::G2.is_g1());
        assert!(!RaceClass::Open.is_g1());
        assert!(!RaceClass::Maiden.is_g1());
    }

    #[test]
    fn display_uses_short_jp_labels() {
        assert_eq!(RaceClass::G1.to_string(), "G1");
        assert_eq!(RaceClass::G2.to_string(), "G2");
        assert_eq!(RaceClass::G3.to_string(), "G3");
        assert_eq!(RaceClass::Listed.to_string(), "L");
        assert_eq!(RaceClass::Open.to_string(), "OP");
        assert_eq!(RaceClass::Win3.to_string(), "3勝");
        assert_eq!(RaceClass::Win2.to_string(), "2勝");
        assert_eq!(RaceClass::Win1.to_string(), "1勝");
        assert_eq!(RaceClass::Maiden.to_string(), "未勝利");
        assert_eq!(RaceClass::NewComer.to_string(), "新馬");
    }

    #[test]
    fn from_label_grade_takes_priority_over_condition() {
        // カード結合ラベル（title にグレード・RaceData02 に「オープン」）でグレードが勝つ。
        assert_eq!(
            RaceClass::from_label("安田記念(G1) 東京 オープン 定量"),
            Some(RaceClass::G1)
        );
    }

    #[test]
    fn from_label_reads_conditions() {
        assert_eq!(RaceClass::from_label("2歳新馬"), Some(RaceClass::NewComer));
        assert_eq!(RaceClass::from_label("3歳未勝利"), Some(RaceClass::Maiden));
        assert_eq!(RaceClass::from_label("3勝クラス"), Some(RaceClass::Win3));
        assert_eq!(RaceClass::from_label("2勝クラス"), Some(RaceClass::Win2));
        assert_eq!(RaceClass::from_label("1勝クラス"), Some(RaceClass::Win1));
        // netkeiba の RaceData02 は全角数字でレンダするため、全角の n勝クラスも拾う。
        assert_eq!(RaceClass::from_label("３勝クラス"), Some(RaceClass::Win3));
        assert_eq!(RaceClass::from_label("２勝クラス"), Some(RaceClass::Win2));
        assert_eq!(RaceClass::from_label("１勝クラス"), Some(RaceClass::Win1));
        assert_eq!(RaceClass::from_label("オープン"), Some(RaceClass::Open));
        assert_eq!(
            RaceClass::from_label("霞ステークス(L)"),
            Some(RaceClass::Listed)
        );
        // 全角括弧のリステッド表記も拾う。
        assert_eq!(
            RaceClass::from_label("霞ステークス（L）"),
            Some(RaceClass::Listed)
        );
    }

    #[test]
    fn from_label_maiden_not_confused_with_win() {
        // 「未勝利」は「n勝」判定より前に拾い、Win* に落ちない。
        assert_eq!(RaceClass::from_label("3歳未勝利"), Some(RaceClass::Maiden));
    }

    #[test]
    fn from_label_none_when_unknown() {
        assert_eq!(RaceClass::from_label("○○特別"), None);
        assert_eq!(RaceClass::from_label(""), None);
    }

    #[test]
    fn is_graded_only_g1_g2_g3() {
        assert!(RaceClass::G1.is_graded());
        assert!(RaceClass::G2.is_graded());
        assert!(RaceClass::G3.is_graded());
        assert!(!RaceClass::Listed.is_graded());
        assert!(!RaceClass::Open.is_graded());
        assert!(!RaceClass::Maiden.is_graded());
    }

    #[test]
    fn ordering_is_g1_highest() {
        assert!(RaceClass::G1 > RaceClass::G2);
        assert!(RaceClass::G3 > RaceClass::Open);
        assert!(RaceClass::NewComer < RaceClass::Win1);
        assert_eq!(
            *[RaceClass::Maiden, RaceClass::G1, RaceClass::Open]
                .iter()
                .max()
                .unwrap(),
            RaceClass::G1
        );
    }

    #[test]
    fn db_slug_round_trips() {
        for c in [
            RaceClass::G1,
            RaceClass::G2,
            RaceClass::G3,
            RaceClass::Listed,
            RaceClass::Open,
            RaceClass::Win3,
            RaceClass::Win2,
            RaceClass::Win1,
            RaceClass::Maiden,
            RaceClass::NewComer,
        ] {
            assert_eq!(RaceClass::try_from(c.as_str()).unwrap(), c);
        }
        assert!(RaceClass::try_from("bogus").is_err());
    }
}
