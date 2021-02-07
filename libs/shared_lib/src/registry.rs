use crate::net::EntityNetId;
use bevy::prelude::*;
use std::{collections::HashMap, hash::Hash};

#[derive(Default)]
pub struct Registry<K: Copy + Hash + IncrementId + Eq, V: Copy + Hash> {
    counter: K,
    value_by_id: HashMap<K, V>,
    id_by_value: HashMap<V, K>,
}

impl<K: Copy + Hash + IncrementId + Eq, V: Copy + Hash + Eq> Registry<K, V> {
    pub fn register(&mut self, value: V) -> K {
        let net_id = self.counter;
        self.counter.increment();
        self.value_by_id.insert(net_id, value);
        self.id_by_value.insert(value, net_id);
        net_id
    }

    pub fn remove_by_value(&mut self, value: V) {
        if let Some(id) = self.id_by_value.remove(&value) {
            self.value_by_id.remove(&id);
        }
    }

    pub fn remove_by_id(&mut self, id: K) {
        if let Some(value) = self.value_by_id.remove(&id) {
            self.id_by_value.remove(&value);
        }
    }

    pub fn get_value(&self, id: K) -> Option<V> {
        self.value_by_id.get(&id).copied()
    }

    pub fn get_id(&self, value: V) -> Option<K> {
        self.id_by_value.get(&value).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.value_by_id.iter()
    }
}

pub trait IncrementId {
    fn increment(&mut self);
}

pub struct EntityRegistry<K: Copy + Hash + Eq> {
    entity_by_id: HashMap<K, Entity>,
    id_by_entity: HashMap<Entity, K>,
}

impl<K: Copy + Hash + Eq> EntityRegistry<K> {
    pub fn register(&mut self, id: K, entity: Entity) {
        self.entity_by_id.insert(id, entity);
        self.id_by_entity.insert(entity, id);
    }

    pub fn remove_by_entity(&mut self, entity: Entity) {
        if let Some(id) = self.id_by_entity.remove(&entity) {
            self.entity_by_id.remove(&id);
        }
    }

    pub fn remove_by_id(&mut self, id: K) {
        if let Some(entity) = self.entity_by_id.remove(&id) {
            self.id_by_entity.remove(&entity);
        }
    }

    pub fn get_entity(&self, id: K) -> Option<Entity> {
        self.entity_by_id.get(&id).copied()
    }

    pub fn get_id(&self, entity: Entity) -> Option<K> {
        self.id_by_entity.get(&entity).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &Entity)> {
        self.entity_by_id.iter()
    }
}
