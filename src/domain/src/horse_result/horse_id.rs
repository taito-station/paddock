use crate::string::define_string;

// netkeiba の馬 ID（例 "2019104567"）。同名馬を一意に切り分ける識別子で、
// 近走取得の供給元 URL `/horse/result/<horse_id>/` のキーになる。
define_string!(HorseId, max = 32);
