pub fn dedup_by_key_unsorted<T, F, K>(vec: &mut Vec<T>, mut key: F)
where
    F: FnMut(&T) -> K,
    K: PartialEq,
{
    let mut new = Vec::new();
    for el in std::mem::take(vec) {
        let el_key = key(&el);
        if !new.iter().any(|i| key(i) == el_key) {
            new.push(el);
        }
    }
    std::mem::swap(&mut new, vec);
}
