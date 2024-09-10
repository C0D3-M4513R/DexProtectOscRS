use std::cmp::Ordering;

pub struct KeyValue<K,V> {
    pub key: K,
    pub value: V,
    pub uuid: uuid::Uuid,
}

impl<K, V> KeyValue<K, V> {
    #[inline]
    pub(crate) fn new(key: K, value: V, uuid: uuid::Uuid) -> KeyValue<K, V> {
        KeyValue { key, value, uuid}
    }
}

impl<K: PartialEq<K>, V> PartialEq<Self> for KeyValue<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.key.eq(&other.key)
    }
}

impl<K:Eq,V> Eq for KeyValue<K,V>{
}

impl<K: PartialOrd<K>, V> PartialOrd<Self> for KeyValue<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.key.partial_cmp(&other.key)
    }

    fn lt(&self, other: &Self) -> bool {
        self.key.lt(&other.key)
    }

    fn le(&self, other: &Self) -> bool {
        self.key.le(&other.key)
    }

    fn gt(&self, other: &Self) -> bool {
        self.key.gt(&other.key)
    }

    fn ge(&self, other: &Self) -> bool {
        self.key.ge(&other.key)
    }
}

impl<K:Ord,V> Ord for KeyValue<K,V> {

    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}
