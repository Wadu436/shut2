use dotenv::dotenv;

use lazy_static::lazy_static;
use regex::Regex;

use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::{macros::*, CommandResult};
use serenity::framework::StandardFramework;
use serenity::model::channel::Message;
use serenity::model::guild::Guild;
use serenity::model::id::ChannelId;
use serenity::model::{id::GuildId, prelude::Ready};
use serenity::prelude::{GatewayIntents, Mentionable, RwLock, TypeMapKey};
use sqlite::Value;

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Settings {
    connection: Mutex<sqlite::Connection>,

    banned_channels: HashSet<ChannelId>,
}

impl Settings {
    fn load() -> Self {
        fs::create_dir_all("data").unwrap();

        let mut banned_channels = HashSet::new();
        let connection = sqlite::open("data/settings.sqlite").unwrap();

        // Create schema if it doesn't exist
        connection
            .execute("CREATE TABLE IF NOT EXISTS banned_channels (channel_id INTEGER NOT NULL);")
            .unwrap();

        // Load banned_channels
        {
            let mut cursor = connection
                .prepare("SELECT * FROM banned_channels")
                .unwrap()
                .into_cursor();

            while let Some(row) = cursor.next().unwrap() {
                if let Value::Integer(channel_id) = row[0] {
                    banned_channels.insert(ChannelId(channel_id as u64));
                }
            }
        }

        return Settings {
            connection: Mutex::new(connection),
            banned_channels,
        };
    }

    fn toggle_channel(&mut self, channel: ChannelId) -> bool {
        let conn_lock = self.connection.lock().unwrap();
        if self.banned_channels.remove(&channel) {
            let mut statement = conn_lock
                .prepare("DELETE FROM banned_channels WHERE channel_id = ?")
                .unwrap();
            statement.bind(1, channel.0 as i64).unwrap();
            statement.next().unwrap();

            return true;
        } else {
            let mut statement = conn_lock
                .prepare("INSERT INTO banned_channels VALUES (?)")
                .unwrap();
            statement.bind(1, channel.0 as i64).unwrap();
            statement.next().unwrap();

            self.banned_channels.insert(channel);
            return false;
        }
    }
}

impl TypeMapKey for Settings {
    type Value = Arc<RwLock<Settings>>;
}

#[group]
#[commands(toggle_channel)]
#[only_in(guilds)]
#[required_permissions(MANAGE_MESSAGES)]
struct General;

struct Handler;
#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }

    async fn cache_ready(&self, ctx: Context, guilds: Vec<GuildId>) {
        println!("Cache built successfully!");
        println!("Guilds:");
        for guildid in guilds {
            let guild = Guild::get(&ctx.http, guildid).await.unwrap();
            println!("\t{}", guild.name)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    let settings = Arc::new(RwLock::new(Settings::load()));

    let framework = StandardFramework::new()
        .configure(|c| c.with_whitespace(true).prefix('~'))
        .normal_message(normal_message)
        .group(&GENERAL_GROUP);

    // Login with a bot token from the environment
    let token = env::var("DISCORD_TOKEN").expect("token");
    let mut client = Client::builder(
        token,
        GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_PRESENCES
            | GatewayIntents::GUILD_MEMBERS,
    )
    .framework(framework)
    .event_handler(Handler)
    .await
    .expect("Error creating client");

    {
        let mut data = client.data.write().await;
        data.insert::<Settings>(settings);
    }

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }

    Ok(())
}

#[hook]
async fn normal_message(ctx: &Context, msg: &Message) {
    // Don't remove bot messages
    if msg.author.bot {
        return;
    }

    // Hyperlink regex
    lazy_static! {
        static ref LINK_RE: Regex = Regex::new(r#"https?://(www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&/=]*)"#).unwrap();
    }
    let link = LINK_RE.is_match(&msg.content);

    // More than 1 attachment or link
    if link || msg.attachments.len() > 0 {
        return;
    }

    // Acquire data lock
    let settings_lock = {
        let data = ctx.data.read().await;
        data.get::<Settings>()
            .expect("Expected Settings in TypeMap.")
            .clone()
    };

    // Acquire settings lock + check if message is in banned channel
    let in_banned_channel = {
        let settings = settings_lock.read().await;
        settings.banned_channels.contains(&msg.channel_id)
    };

    // Message needs to be in banned channel to delete it
    if !in_banned_channel {
        return;
    }

    // Delete the message
    msg.delete(&ctx).await.unwrap();

    let reply_msg = msg
        .channel_id
        .say(&ctx, format!("{} SHUT!", msg.author.mention()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;

    reply_msg.delete(&ctx).await.unwrap();
}

#[command]
async fn toggle_channel(ctx: &Context, msg: &Message) -> CommandResult {
    let channel = msg.channel(&ctx).await?.guild().ok_or("Not in guild")?;

    // Acquire data lock
    let settings_lock = {
        let data = ctx.data.read().await;
        data.get::<Settings>()
            .expect("Expected Settings in TypeMap.")
            .clone()
    };

    // Acquire settings lock + check if message is in banned channel
    let channel_was_banned = {
        let mut settings = settings_lock.write().await;
        settings.toggle_channel(msg.channel_id)
    };

    if channel_was_banned {
        msg.reply(
            ctx,
            format!(
                "SHUT will stop removing messages from {}",
                channel.mention()
            ),
        )
        .await?;
    } else {
        msg.reply(
            ctx,
            format!(
                "SHUT will now remove non-media messages from {}",
                channel.mention()
            ),
        )
        .await?;
    }

    Ok(())
}
