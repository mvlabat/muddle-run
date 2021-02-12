use std::collections::VecDeque;

use crate::looped_counter::WrappedCounter;

pub type FrameNumber = WrappedCounter<u16>;

pub struct Framebuffer<T> {
    start_frame: FrameNumber,
    /// Stores a frame number as the first element of the tuple.
    buffer: VecDeque<T>,
    limit: FrameNumber,
}

impl<T> Framebuffer<T> {
    pub fn new(start_frame: FrameNumber, limit: u16) -> Self {
        if limit < 1 {
            panic!("Framebuffer limit can't be lesser than 1");
        }
        Self {
            start_frame,
            buffer: VecDeque::new(),
            limit: FrameNumber::new(limit),
        }
    }

    pub fn limit(&self) -> u16 {
        self.limit.value()
    }

    pub fn set_limit(&mut self, limit: u16) {
        if limit < 1 {
            panic!("Framebuffer limit can't be lesser than 1");
        }
        self.limit = FrameNumber::new(limit);
        for _ in self.limit.value() as usize..self.buffer.len() {
            self.start_frame += FrameNumber::new(1);
            self.buffer.pop_front();
        }
    }

    pub fn push(&mut self, value: T) {
        if self.buffer.len() == self.limit.value() as usize {
            self.start_frame += FrameNumber::new(1);
            self.buffer.pop_front();
        }
        self.buffer.push_back(value);
    }

    pub fn get(&self, frame_number: FrameNumber) -> Option<&T> {
        self.buffer
            .get((frame_number - self.start_frame).value() as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FrameNumber, &T)> {
        let start_frame = self.start_frame;
        self.buffer
            .iter()
            .enumerate()
            .map(move |(i, v)| (FrameNumber::new(i as u16) + start_frame, v))
    }

    pub fn can_insert(&self, frame_number: FrameNumber) -> bool {
        let frame_len = FrameNumber::new(self.buffer.len() as u16);
        frame_number + self.limit >= self.start_frame + frame_len
    }
}

impl<T: Default> Framebuffer<T> {
    pub fn insert(&mut self, frame_number: FrameNumber, value: T) {
        let frame_len = FrameNumber::new(self.buffer.len() as u16);
        if !self.can_insert(frame_number) {
            panic!("Inserting for a frame {} would remove future history (start_frame: {}, limit: {}, len: {})", frame_number, self.start_frame, self.limit, frame_len);
        }

        if frame_number < self.start_frame {
            for _ in frame_number + FrameNumber::new(1)..self.start_frame {
                self.buffer.push_front(T::default());
            }
            self.buffer.push_front(value);
            self.start_frame = frame_number;
            return;
        }

        let frame_len = FrameNumber::new(self.buffer.len() as u16);
        let end_frame = self.start_frame + frame_len - FrameNumber::new(1);
        if self.buffer.is_empty() {
            self.start_frame = frame_number;
            self.push(value);
        } else if frame_number >= end_frame + self.limit {
            self.buffer.clear();
            self.start_frame = frame_number;
            self.push(value);
        } else if frame_number <= end_frame {
            let offset = frame_len - FrameNumber::new(1) - (end_frame - frame_number);
            self.buffer[offset.value() as usize] = value;
        } else {
            for _ in end_frame + FrameNumber::new(1)..frame_number {
                self.push(T::default());
            }
            self.push(value);
        }
    }
}

impl<T> Framebuffer<Option<T>> {
    /// If the value is `None`, looks behind to find the closest existing value.
    /// If `frame_number` is out of the stored range, returns `None`.
    /// Returns a corresponding `FrameNumber` as the first tuple element.
    pub fn get_with_interpolation(&self, frame_number: FrameNumber) -> Option<(FrameNumber, &T)> {
        let end_frame = self.start_frame + FrameNumber::new(self.buffer.len() as u16 - 1);
        if frame_number > end_frame {
            return None;
        }
        let skip = end_frame - frame_number;
        self.buffer
            .iter()
            .rev()
            .skip(skip.value() as usize)
            .enumerate()
            .find(|(_, v)| v.is_some())
            .map(|(i, v)| {
                (
                    frame_number - FrameNumber::new(i as u16),
                    v.as_ref().unwrap(),
                )
            })
    }

    pub fn iter_with_interpolation(&self) -> impl Iterator<Item = (FrameNumber, &T)> {
        let mut last_some: Option<&T> = None;
        let start_frame = self.start_frame;
        self.buffer
            .iter()
            .enumerate()
            .skip_while(|(_, v)| v.is_none())
            .map(move |(i, v)| {
                let frame_number = FrameNumber::new(i as u16) + start_frame;
                let value = if let Some(value) = v {
                    last_some = Some(value);
                    value
                } else {
                    last_some.unwrap()
                };
                (frame_number, value)
            })
    }

    pub fn iter_with_extrapolation(
        &self,
        end_frame: FrameNumber,
    ) -> impl Iterator<Item = (FrameNumber, &T)> {
        let mut last_some: Option<&T> = None;
        let start_frame = self.start_frame;
        let first_some_index = self
            .buffer
            .iter()
            .position(|v| v.is_some())
            .unwrap_or_else(|| self.buffer.len());

        (start_frame..=end_frame)
            .skip(first_some_index)
            .map(move |frame_number| {
                let i = (frame_number - start_frame).value() as usize;
                let value = if let Some(value) = self.buffer.get(i).and_then(|v| v.as_ref()) {
                    last_some = Some(value);
                    value
                } else {
                    last_some.unwrap()
                };
                (frame_number, value)
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::{framebuffer::Framebuffer, FrameNumber};

    #[test]
    fn test_push() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        assert_eq!(buffer.buffer.len(), 2);
        assert_eq!(buffer.start_frame, FrameNumber::new(1));
        assert_eq!(buffer.limit, FrameNumber::new(2));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(1), &2usize),
                (FrameNumber::new(2), &3usize)
            ]
        );
    }

    #[test]
    fn test_set_same_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 1);
        buffer.push(1);
        buffer.set_limit(1);

        assert_eq!(buffer.buffer.len(), 1);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(1));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![(FrameNumber::new(0), &1usize)]
        );
    }

    #[test]
    fn test_set_lesser_limit_empty() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.set_limit(1);

        assert_eq!(buffer.buffer.len(), 0);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(1));
        assert_eq!(buffer.iter().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    fn test_set_lesser_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.push(1);
        buffer.push(2);
        buffer.set_limit(1);

        assert_eq!(buffer.buffer.len(), 1);
        assert_eq!(buffer.start_frame, FrameNumber::new(1));
        assert_eq!(buffer.limit, FrameNumber::new(1));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![(FrameNumber::new(1), &2usize)]
        );
    }

    #[test]
    fn test_set_larger_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 3);
        buffer.push(1);
        buffer.push(2);
        buffer.set_limit(3);

        assert_eq!(buffer.buffer.len(), 2);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(3));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(0), &1usize),
                (FrameNumber::new(1), &2usize)
            ]
        );
    }

    #[test]
    fn test_insert_back() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 3);
        buffer.push(1);

        buffer.insert(FrameNumber::new(0) - FrameNumber::new(2), 2);
        assert_eq!(buffer.buffer.len(), 3);
        assert_eq!(buffer.start_frame, FrameNumber::new(u16::MAX - 1));
        assert_eq!(buffer.limit, FrameNumber::new(3));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(u16::MAX - 1), &2usize),
                (FrameNumber::new(u16::MAX), &0usize),
                (FrameNumber::new(0), &1usize)
            ]
        );
    }

    #[test]
    #[should_panic(
        expected = "Inserting for a frame 0 would remove future history (start_frame: 3, limit: 3, len: 1)"
    )]
    fn test_insert_back_panic() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(3), 3);
        buffer.push(1);
        buffer.insert(FrameNumber::new(0), 2);
    }

    #[test]
    fn test_insert_into_empty() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.insert(FrameNumber::new(1), 1);

        assert_eq!(buffer.buffer.len(), 1);
        assert_eq!(buffer.start_frame, FrameNumber::new(1));
        assert_eq!(buffer.limit, FrameNumber::new(2));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![(FrameNumber::new(1), &1usize)]
        );
    }

    #[test]
    fn test_insert_far_over_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.push(1);
        buffer.push(2);
        buffer.insert(FrameNumber::new(3), 3);

        assert_eq!(buffer.buffer.len(), 1);
        assert_eq!(buffer.start_frame, FrameNumber::new(3));
        assert_eq!(buffer.limit, FrameNumber::new(2));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![(FrameNumber::new(3), &3usize)]
        );
    }

    #[test]
    fn insert_into_existing() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.push(1);
        buffer.push(2);
        buffer.insert(FrameNumber::new(1), 3);

        assert_eq!(buffer.buffer.len(), 2);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(2));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(0), &1usize),
                (FrameNumber::new(1), &3usize)
            ]
        );
    }

    #[test]
    fn insert_next() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 2);
        buffer.push(1);
        buffer.insert(FrameNumber::new(1), 3);

        assert_eq!(buffer.buffer.len(), 2);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(2));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(0), &1usize),
                (FrameNumber::new(1), &3usize)
            ]
        );
    }

    #[test]
    fn test_insert_inside_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 3);
        buffer.push(1);
        buffer.insert(FrameNumber::new(2), 3);

        assert_eq!(buffer.buffer.len(), 3);
        assert_eq!(buffer.start_frame, FrameNumber::new(0));
        assert_eq!(buffer.limit, FrameNumber::new(3));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(0), &1usize),
                (FrameNumber::new(1), &0usize),
                (FrameNumber::new(2), &3usize)
            ]
        );
    }

    #[test]
    fn test_insert_outside_limit() {
        let mut buffer = Framebuffer::<usize>::new(FrameNumber::new(0), 3);
        buffer.push(1);
        buffer.push(2);
        buffer.push(3);
        buffer.insert(FrameNumber::new(4), 5);

        assert_eq!(buffer.buffer.len(), 3);
        assert_eq!(buffer.start_frame, FrameNumber::new(2));
        assert_eq!(buffer.limit, FrameNumber::new(3));
        assert_eq!(
            buffer.iter().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(2), &3usize),
                (FrameNumber::new(3), &0usize),
                (FrameNumber::new(4), &5usize)
            ]
        );
    }

    #[test]
    fn test_iter_with_interpolation() {
        let mut buffer = Framebuffer::<Option<usize>>::new(FrameNumber::new(0), 5);
        buffer.push(None);
        buffer.push(Some(1));
        buffer.push(None);
        buffer.push(Some(2));
        buffer.push(None);

        assert_eq!(
            buffer.iter_with_interpolation().collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(1), &1usize),
                (FrameNumber::new(2), &1usize),
                (FrameNumber::new(3), &2usize),
                (FrameNumber::new(4), &2usize)
            ]
        )
    }

    #[test]
    fn test_iter_with_extrapolation() {
        let mut buffer = Framebuffer::<Option<usize>>::new(FrameNumber::new(0), 5);
        buffer.push(None);
        buffer.push(Some(1));
        buffer.push(None);
        buffer.push(Some(2));
        buffer.push(None);

        assert_eq!(
            buffer
                .iter_with_extrapolation(FrameNumber::new(5))
                .collect::<Vec<_>>(),
            vec![
                (FrameNumber::new(1), &1usize),
                (FrameNumber::new(2), &1usize),
                (FrameNumber::new(3), &2usize),
                (FrameNumber::new(4), &2usize),
                (FrameNumber::new(5), &2usize)
            ]
        )
    }
}
