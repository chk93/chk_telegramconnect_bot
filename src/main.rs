use std::collections::HashMap;
use std::sync::Arc;
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, Me, Message, ParseMode, User},
    utils::command::BotCommands,
};
use tokio::sync::Mutex;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    Start,
    Help,
    Contacts,
    Cancel,
}

#[derive(Clone, Debug)]
enum UserState {
    WaitingForPurpose,
    WaitingForMessage { purpose: String },
}

type UserStates = Arc<Mutex<HashMap<i64, UserState>>>;

const BOT_TOKEN: &str = "BOT_TOKEN"; // <== PASTE UR TOKEN HERE
const ADMIN_CHAT_ID: i64 = 123456789; // <++ PASTE UR TELEGRAMM ID HERE (@getmyid_bot)

const CONTACTS_TEXT: &str = r#"
┌─────────────────────
│ <b>Telegram:</b> @your_username
│ <b>GitHub:</b> github.com/your_username
│ <b>Discord:</b> your_discord_username
│ <b>Email:</b> your.email@example.com       
└─────────────────────
"#; // or add ur links

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("starting bot");

    if ADMIN_CHAT_ID == 123456789 {
        log::error!("change ADMIN_CHAT_ID to your actual id");
        return;
    }
    if BOT_TOKEN == "YOUR_BOT_TOKEN_HERE" {
        log::error!("change BOT_TOKEN to your actual bot token");
        return;
    }

    let bot = Bot::new(BOT_TOKEN);
    let user_states: UserStates = Arc::new(Mutex::new(HashMap::new()));

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(message_handler))
        .branch(Update::filter_callback_query().endpoint(callback_handler));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![user_states])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn message_handler(
    bot: Bot,
    msg: Message,
    me: Me,
    user_states: UserStates,
) -> ResponseResult<()> {
    let user_id = msg.chat.id.0 as i64;
    let text = msg.text();

    let state = {
        let states = user_states.lock().await;
        states.get(&user_id).cloned()
    };

    log::info!("message from {}, state: {:?}", user_id, state);

    if let Some(text) = text {
        if let Ok(cmd) = Command::parse(text, me.username()) {
            match cmd {
                Command::Start => {
                    user_states.lock().await.remove(&user_id);
                    send_welcome(&bot, user_id, &user_states).await?;
                }
                Command::Help => {
                    user_states.lock().await.remove(&user_id);
                    send_help(&bot, user_id).await?;
                }
                Command::Contacts => {
                    send_contacts(&bot, user_id).await?;
                }
                Command::Cancel => {
                    user_states.lock().await.remove(&user_id);
                    bot.send_message(
                        ChatId(user_id),
                        "cancelled. /start to begin again"
                    ).await?;
                }
            }
            return Ok(());
        }
    }

    match state {
        None => {
            send_welcome(&bot, user_id, &user_states).await?;
        }
        Some(UserState::WaitingForPurpose) => {
            bot.send_message(
                ChatId(user_id),
                "use the buttons below to select purpose"
            ).await?;
            send_welcome(&bot, user_id, &user_states).await?;
        }
        Some(UserState::WaitingForMessage { purpose }) => {
            let message_text = text.unwrap_or("[not text]").to_string();
            
            log::info!("sending notification to admin, purpose: {}, message: {}", purpose, message_text);
            
            notify_admin(&bot, &msg, &purpose, &message_text).await?;
            user_states.lock().await.remove(&user_id);
            
            bot.send_message(
                ChatId(user_id),
                "got it. i'll get back to you soon."
            ).await?;
        }
    }

    Ok(())
}

async fn callback_handler(
    bot: Bot,
    q: CallbackQuery,
    user_states: UserStates,
) -> ResponseResult<()> {
    let user_id = q.from.id.0 as i64;
    let data = match q.data {
        Some(data) => data,
        None => return Ok(()),
    };

    log::info!("callback from {}, data: {}", user_id, data);

    match data.as_str() {
        "personal" => {
            bot.answer_callback_query(&q.id).await?;
            
            user_states.lock().await.insert(
                user_id,
                UserState::WaitingForMessage {
                    purpose: "personal".to_string(),
                },
            );
            
            log::info!("user {} chose personal, state updated", user_id);
            
            bot.send_message(
                ChatId(user_id),
                "purpose: personal\n\ntype your message or press skip"
            )
            .reply_markup(skip_keyboard())
            .await?;
        }
        "business" => {
            bot.answer_callback_query(&q.id).await?;
            
            user_states.lock().await.insert(
                user_id,
                UserState::WaitingForMessage {
                    purpose: "business".to_string(),
                },
            );
            
            log::info!("user {} chose business, state updated", user_id);
            
            bot.send_message(
                ChatId(user_id),
                "purpose: business\n\ntype your message or press skip"
            )
            .reply_markup(skip_keyboard())
            .await?;
        }
        "skip" => {
            bot.answer_callback_query(&q.id).await?;
            
            log::info!("user {} pressed skip", user_id);
            
            let purpose = {
                let states = user_states.lock().await;
                match states.get(&user_id) {
                    Some(UserState::WaitingForMessage { purpose }) => {
                        log::info!("found purpose: {}", purpose);
                        purpose.clone()
                    },
                    state => {
                        log::error!("unexpected state for user {}: {:?}", user_id, state);
                        bot.send_message(ChatId(user_id), "something went wrong, /start again").await?;
                        return Ok(());
                    }
                }
            };
            
            log::info!("sending skip notification to admin, purpose: {}", purpose);
            
            notify_admin_skip(&bot, &q.from, &purpose).await?;
            user_states.lock().await.remove(&user_id);
            
            bot.send_message(
                ChatId(user_id),
                "skipped. i'll get back to you soon."
            ).await?;
        }
        "contacts" => {
            bot.answer_callback_query(&q.id).await?;
            send_contacts(&bot, user_id).await?;
        }
        _ => {
            bot.answer_callback_query(&q.id).await?;
        }
    }

    Ok(())
}

async fn send_welcome(bot: &Bot, user_id: i64, user_states: &UserStates) -> ResponseResult<()> {
    user_states.lock().await.insert(
        user_id,
        UserState::WaitingForPurpose,
    );
    
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("□ personal", "personal"),
            InlineKeyboardButton::callback("■ business", "business"),
        ],
        vec![
            InlineKeyboardButton::callback("≡ contacts", "contacts"),
        ],
    ]);

    bot.send_message(ChatId(user_id), "select purpose:")
        .reply_markup(keyboard)
        .await?;
    
    Ok(())
}

async fn send_help(bot: &Bot, user_id: i64) -> ResponseResult<()> {
    bot.send_message(
        ChatId(user_id),
        "/start - begin\n/contacts - my contacts\n/cancel - stop current action"
    ).await?;
    Ok(())
}

async fn send_contacts(bot: &Bot, user_id: i64) -> ResponseResult<()> {
    bot.send_message(ChatId(user_id), CONTACTS_TEXT)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

fn skip_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("≡ skip", "skip")]
    ])
}

async fn notify_admin(bot: &Bot, msg: &Message, purpose: &str, message_text: &str) -> ResponseResult<()> {
    let user = msg.from.as_ref().unwrap();
    let username = user.username.as_deref().unwrap_or("no_username");
    let full_name = user.full_name();
    
    let text = format!(
        "new message\n\nfrom: @{} ({})\nid: `{}`\npurpose: {}\n\nmessage:\n{}",
        username,
        full_name,
        user.id.0,
        purpose,
        escape_markdown(message_text)
    );

    log::info!("sending to admin ({}): {}", ADMIN_CHAT_ID, text);
    
    bot.send_message(ChatId(ADMIN_CHAT_ID), text)
        .await?;
    
    Ok(())
}

async fn notify_admin_skip(bot: &Bot, user: &User, purpose: &str) -> ResponseResult<()> {
    let username = user.username.as_deref().unwrap_or("no_username");
    let full_name = user.full_name();
    
    let text = format!(
        "new message (skipped)\n\nfrom: @{} ({})\nid: `{}`\npurpose: {}\n\nmessage: _skipped_",
        username,
        full_name,
        user.id.0,
        purpose
    );

    log::info!("sending skip to admin ({}): {}", ADMIN_CHAT_ID, text);
    
    bot.send_message(ChatId(ADMIN_CHAT_ID), text)
        .await?;
    
    Ok(())
}

fn escape_markdown(s: &str) -> String {
    s.replace('_', "\\_")
        .replace('*', "\\*")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('~', "\\~")
        .replace('`', "\\`")
        .replace('>', "\\>")
        .replace('#', "\\#")
        .replace('+', "\\+")
        .replace('-', "\\-")
        .replace('=', "\\=")
        .replace('|', "\\|")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('.', "\\.")
        .replace('!', "\\!")
}