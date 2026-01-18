
mod exec_handle;
mod single_threaded_crud;

pub struct MonolithicPreemptiveExecutor;

// impl<A, S> TExecutor<A, S> for MonolithicPreemptiveExecutor
// where
//     A: Application<S>,
// {
//     type ExecutionHandle = PreemptiveExecutorHandle<Request<A, S>>;
// 
//     fn init_handle() -> Self::ExecutionHandle {
//         todo!()
//     }
// }
