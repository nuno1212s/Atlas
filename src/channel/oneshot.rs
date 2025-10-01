/*
 One shot channels
 @{
*/
use crate::channel::oneshot_spsc;

pub type OneShotTx<T> = oneshot_spsc::OneShotTx<T>;

pub type OneShotRx<T> = oneshot_spsc::OneShotRx<T>;

#[inline]
pub fn new_oneshot_channel<T>() -> (OneShotTx<T>, OneShotRx<T>) {
    oneshot_spsc::new_oneshot()
}
