use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSFileCoordinator, NSFilePresenter, NSObject, NSObjectProtocol, NSOperationQueue, NSURL,
};

struct PresenterIvars {
    source: Retained<NSURL>,
    directory: Retained<NSURL>,
    queue: Retained<NSOperationQueue>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[ivars = PresenterIvars]
    struct RelatedSourcePresenter;

    unsafe impl NSObjectProtocol for RelatedSourcePresenter {}

    unsafe impl NSFilePresenter for RelatedSourcePresenter {
        #[unsafe(method_id(presentedItemURL))]
        fn presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().directory.clone())
        }

        #[unsafe(method_id(presentedItemOperationQueue))]
        fn presented_item_operation_queue(&self) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }

        #[unsafe(method_id(primaryPresentedItemURL))]
        fn primary_presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().source.clone())
        }
    }
);

pub struct RelatedSourceAccess {
    presenter: Retained<RelatedSourcePresenter>,
}

impl RelatedSourceAccess {
    pub fn new(source: &Path) -> Result<Self, String> {
        let directory = source
            .parent()
            .ok_or_else(|| format!("Manim source has no parent directory: {}", source.display()))?;
        let source = NSURL::from_file_path(source)
            .ok_or("could not create a native URL for the Manim source")?;
        let directory = NSURL::from_file_path(directory)
            .ok_or("could not create a native URL for the Manim source directory")?;
        let queue = NSOperationQueue::new();
        queue.setMaxConcurrentOperationCount(1);
        let presenter = RelatedSourcePresenter::alloc().set_ivars(PresenterIvars {
            source,
            directory,
            queue,
        });
        let presenter: Retained<RelatedSourcePresenter> =
            unsafe { msg_send![super(presenter), init] };
        NSFileCoordinator::addFilePresenter(ProtocolObject::from_ref(&*presenter));
        Ok(Self { presenter })
    }
}

impl Drop for RelatedSourceAccess {
    fn drop(&mut self) {
        NSFileCoordinator::removeFilePresenter(ProtocolObject::from_ref(&*self.presenter));
    }
}
