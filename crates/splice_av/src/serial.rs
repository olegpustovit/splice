//! Synchronous owned commands for the offscreen Metal queue.
use super::Error;
use ogpu_core::backend::{self as b, Commands, Queue};
use std::ffi::c_void;

pub(super) struct SerialCommands {
    completed: Option<Commands>,
    queue: Queue,
    failed: bool,
}

impl SerialCommands {
    pub(super) fn new(queue: Queue) -> Self {
        Self {
            completed: None,
            queue,
            failed: false,
        }
    }

    /// Check before advancing a decoder or replacing a writer's pending buffer.
    pub(super) fn check(&self) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Input(
                "The offscreen GPU queue failed. Reopen the composition or restart the export."
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn run(
        &mut self,
        record: &mut dyn FnMut(&Commands, *mut c_void) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.check()?;
        let commands = match self.completed.take() {
            Some(commands) => {
                b::rebegin_commands(&commands).map_err(gpu_error)?;
                commands
            }
            None => b::begin_commands(&self.queue).map_err(gpu_error)?,
        };
        let submitted = (|| {
            let native = b::native_commands(&commands).map_err(gpu_error)?;
            record(&commands, native.mtl_command_buffer)?;
            b::commit(&commands).map_err(gpu_error)
        })();
        let receipt = match submitted {
            Ok(receipt) => receipt,
            Err(error) => {
                self.failed = b::abandon(&commands).is_err();
                return Err(error);
            }
        };
        if let Err(error) = b::complete_through(&self.queue, receipt.epoch) {
            self.failed = true;
            return Err(gpu_error(error));
        }
        self.completed = Some(commands);
        Ok(())
    }
}

pub(super) fn gpu_error(error: ogpu_core::error::Error) -> Error {
    Error::Input(ogpu_core::error::to_string(&error))
}
