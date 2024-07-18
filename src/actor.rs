use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{Channel, Receiver, Sender},
};

pub const MESSAGE_STREAM_SIZE: usize = 16;

pub trait Actor {
    type Message;

    async fn run(&mut self, msg_queue: MessageReceiverFor<Self>);
}

pub type MessageStream<MessageT> = Channel<NoopRawMutex, MessageT, MESSAGE_STREAM_SIZE>;
pub type MessageFor<ActorT> = <ActorT as Actor>::Message;
pub type MessageStreamFor<ActorT> = MessageStream<MessageFor<ActorT>>;

pub type MessageSender<MessageT> = Sender<'static, NoopRawMutex, MessageT, MESSAGE_STREAM_SIZE>;
pub type MessageSenderFor<ActorT> = MessageSender<MessageFor<ActorT>>;

pub type MessageReceiver<MessageT> = Receiver<'static, NoopRawMutex, MessageT, MESSAGE_STREAM_SIZE>;
pub type MessageReceiverFor<ActorT> = MessageReceiver<MessageFor<ActorT>>;

pub type ChannelStorage<MessageT> = static_cell::StaticCell<MessageStream<MessageT>>;
pub type ChannelStorageFor<ActorT> = ChannelStorage<MessageFor<ActorT>>;

macro_rules! actor_spawn {
    ($spawner:expr, $name:ident, $actor_type:ty, $instance:expr) => {{
        const MESSAGE_STREAM_SIZE: usize = crate::actor::MESSAGE_STREAM_SIZE;

        #[embassy_executor::task]
        async fn $name(
            msg_queue: embassy_sync::channel::Receiver<
                'static,
                embassy_sync::blocking_mutex::raw::NoopRawMutex,
                crate::actor::MessageFor<$actor_type>,
                MESSAGE_STREAM_SIZE,
            >,
            mut actor: $actor_type,
        ) {
            actor.run(msg_queue).await;
        }

        static CHANNEL_STORAGE: crate::actor::ChannelStorageFor<$actor_type> =
            crate::actor::ChannelStorageFor::<$actor_type>::new();
        let channel = CHANNEL_STORAGE.init(crate::actor::MessageStreamFor::<$actor_type>::new());

        $spawner
            .spawn($name(channel.receiver(), $instance))
            .expect("Failed to spawn task for actor.");

        channel.sender()
    }};
}
