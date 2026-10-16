use super::*;
use crate::rooms::*;

impl Store {
    fn room_person(&self, p: &Principal, name: &str) -> Result<String> {
        let name = name.trim().to_lowercase();
        if name == "me" || name == p.user_id {
            return Ok(p.user_id.clone());
        }
        let people = self.contacts(p)?;
        let exact: Vec<_> = people
            .iter()
            .filter(|c| c.id == name || c.name.to_lowercase() == name)
            .collect();
        let found = if exact.is_empty() {
            people
                .iter()
                .filter(|c| c.name.to_lowercase().starts_with(&format!("{name} ")))
                .collect::<Vec<_>>()
        } else {
            exact
        };
        validate(
            !found.is_empty(),
            "This person is not available in your workspace. Use litecom_contacts to find them.",
        )?;
        if found.len() != 1 {
            return Err(Error::Conflict(format!(
                "Several people match. Choose an ID: {}",
                found
                    .iter()
                    .map(|c| format!("{} ({})", c.name, c.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        self.check_workspace(&found[0].id)?;
        Ok(found[0].id.clone())
    }
    fn room_key(value: &str) -> Result<()> {
        validate(
            !value.is_empty() && value.len() <= 128,
            "Use an ID between 1 and 128 bytes.",
        )
    }
    pub fn room_create(&self, p: &Principal, input: &CreateChannel) -> Result<Channel> {
        Self::room_key(&input.request_id)?;
        validate(
            !input.members.is_empty() && input.members.len() <= 15,
            "Choose between 1 and 15 people.",
        )?;
        validate(
            input.topic.len() <= 200,
            "Keep the channel topic under 200 bytes.",
        )?;
        let mut members = input
            .members
            .iter()
            .map(|m| self.room_person(p, m))
            .collect::<Result<Vec<_>>>()?;
        members.push(p.user_id.clone());
        members.sort();
        members.dedup();
        for a in &members {
            for b in &members {
                require(
                    self.requests_allowed(a, b)?,
                    "A member has blocked contact with another member.",
                )?;
            }
        }
        let fingerprint = serde_json::to_vec(&(members.clone(), input.topic.trim()))?;
        self.transaction(|| {
            let previous: Option<(String, Vec<u8>)> = self.db.query_row("SELECT id,creation FROM rooms WHERE owner_id=? AND request_key=?",params![p.user_id,input.request_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((id, data)) = previous {
                let original: Vec<u8> = self.vault.open(&data,&format!("room-create:{id}"))?;
                conflict(original == fingerprint, "This request ID was already used for a different channel.")?;
                return self.room(p,&id);
            }
            let count: i64 = self.db.query_row("SELECT COUNT(*) FROM rooms WHERE owner_id=? AND closed=0",[&p.user_id],|r|r.get(0))?;
            validate(count < 500, "Close an unused channel before opening another.")?;
            let id = id();
            let topic = if input.topic.trim().is_empty() {
                members.iter().map(|m|Ok(self.profile_by_id(m)?.name)).collect::<Result<Vec<_>>>()?.join(" · ")
            } else { input.topic.trim().to_owned() };
            self.db.execute("INSERT INTO rooms(id,owner_id,request_key,topic,creation,created_at) VALUES(?,?,?,?,?,?)",params![id,p.user_id,input.request_id,self.vault.seal(&topic,&format!("room-topic:{id}"))?,self.vault.seal(&fingerprint,&format!("room-create:{id}"))?,self.time()])?;
            for member in members { self.db.execute("INSERT INTO room_members(room_id,user_id) VALUES(?,?)",params![id,member])?; }
            self.room(p,&id)
        })
    }
    fn room(&self, p: &Principal, id: &str) -> Result<Channel> {
        let row: Option<(String,Vec<u8>,i64,bool)> = self.db.query_row("SELECT owner_id,topic,created_at,closed FROM rooms WHERE id=? AND EXISTS(SELECT 1 FROM room_members WHERE room_id=rooms.id AND user_id=?)",params![id,p.user_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (owner_id, topic, created_at, closed) = row.ok_or(Error::NotFound)?;
        let members = self.db.prepare("SELECT p.id,p.name FROM room_members m JOIN profiles p ON p.id=m.user_id WHERE m.room_id=? ORDER BY p.name,p.id")?.query_map([id],|r|Ok(Member{id:r.get(0)?,name:r.get(1)?}))?.collect::<std::result::Result<Vec<_>,_>>()?;
        self.check_workspace(&p.user_id)?;
        for member in &members {
            self.check_workspace(&member.id)?;
            for other in &members {
                require(
                    self.requests_allowed(&member.id, &other.id)?,
                    "This channel is unavailable because a member has blocked contact.",
                )?;
            }
        }
        Ok(Channel {
            id: id.into(),
            owner_id,
            topic: self.vault.open(&topic, &format!("room-topic:{id}"))?,
            members,
            created_at,
            closed,
        })
    }
    pub fn room_list(
        &self,
        p: &Principal,
        person: Option<&str>,
        closed: bool,
    ) -> Result<Vec<Channel>> {
        let person = person.map(|n| self.room_person(p, n)).transpose()?;
        let ids = self.db.prepare("SELECT r.id FROM rooms r JOIN room_members m ON m.room_id=r.id WHERE m.user_id=? AND (? OR r.closed=0) ORDER BY r.created_at DESC,r.id")?.query_map(params![p.user_id,closed],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut rooms = Vec::new();
        for id in ids {
            match self.room(p, &id) {
                Ok(room)
                    if person
                        .as_ref()
                        .is_none_or(|id| room.members.iter().any(|m| &m.id == id)) =>
                {
                    rooms.push(room)
                }
                Ok(_) | Err(Error::Forbidden(_)) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(rooms)
    }
    pub fn room_join(&self, p: &Principal, id: &str, input: &JoinChannel) -> Result<String> {
        Self::room_key(&input.session_key)?;
        validate(
            input.label.len() <= 160,
            "Keep the chat label under 160 bytes.",
        )?;
        self.transaction(|| {
            conflict(!self.room(p,id)?.closed,"This channel is closed.")?;
            let previous: Option<String> = self.db.query_row("SELECT id FROM room_participants WHERE room_id=? AND credential_id=? AND session_key=?",params![id,p.credential_id,input.session_key],|r|r.get(0)).optional()?;
            if let Some(participant) = previous {
                self.db.execute("UPDATE room_participants SET left_at=NULL,last_seen_at=? WHERE id=?",params![self.time(),participant])?;
                return Ok(participant);
            }
            let count: i64 = self.db.query_row("SELECT COUNT(*) FROM room_participants WHERE room_id=?",[id],|r|r.get(0))?;
            validate(count < 256,"This channel has reached its chat limit. Open a new channel.")?;
            let participant = crate::crypto::id();
            self.db.execute("INSERT INTO room_participants(id,room_id,user_id,agent_id,credential_id,session_key,label,last_seen_at) VALUES(?,?,?,?,?,?,?,?)",params![participant,id,p.user_id,p.agent_id,p.credential_id,input.session_key,self.vault.seal(&input.label,&format!("room-participant:{participant}"))?,self.time()])?;
            Ok(participant)
        })
    }
    fn room_participant(
        &self,
        p: &Principal,
        room: &str,
        participant: &str,
        active: bool,
    ) -> Result<()> {
        let valid: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM room_participants WHERE id=? AND room_id=? AND user_id=? AND credential_id=? AND (?=0 OR left_at IS NULL))",params![participant,room,p.user_id,p.credential_id,active],|r|r.get(0))?;
        require(
            valid,
            "Join this channel from this chat before sending or waiting.",
        )
    }
    pub fn room_page(&self, p: &Principal, id: &str, after: i64) -> Result<ChannelPage> {
        validate(after >= 0, "The message cursor must not be negative.")?;
        let channel = self.room(p, id)?;
        let participants = self.db.prepare("SELECT c.id,c.user_id,p.name,c.agent_id,c.label,c.last_seen_at,c.left_at IS NOT NULL FROM room_participants c JOIN profiles p ON p.id=c.user_id WHERE c.room_id=? ORDER BY c.last_seen_at,c.id")?.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,i64>(5)?,r.get::<_,bool>(6)?)))?.map(|r|{
            let (id,user_id,name,agent_id,label,last_seen_at,left) = r?;
            Ok(Participant{label:self.vault.open(&label,&format!("room-participant:{id}"))?,id,user_id,name,agent_id,last_seen_at,left})
        }).collect::<Result<Vec<_>>>()?;
        let mut messages = self.db.prepare("SELECT sequence,participant_id,text,created_at FROM room_messages WHERE room_id=? AND sequence>? ORDER BY sequence LIMIT 101")?.query_map(params![id,after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,i64>(3)?)))?.map(|r|{
            let (sequence,participant_id,text,created_at) = r?;
            Ok(Message{sequence,participant_id,text:self.vault.open(&text,&format!("room-message:{id}:{sequence}"))?,created_at})
        }).collect::<Result<Vec<_>>>()?;
        let has_more = messages.len() > 100;
        messages.truncate(100);
        let next_cursor = messages.last().map_or(after, |m| m.sequence);
        Ok(ChannelPage {
            channel,
            participants,
            messages,
            next_cursor,
            has_more,
        })
    }
    pub fn room_sync(&self, p: &Principal, room: &str, input: &ChannelSync) -> Result<ChannelPage> {
        validate(input.after >= 0, "The message cursor must not be negative.")?;
        validate(
            input.message.is_some() == input.message_id.is_some(),
            "Sending requires both message and message_id.",
        )?;
        self.transaction(|| {
            let channel = self.room(p,room)?;
            self.room_participant(p,room,&input.participant_id,true)?;
            if let (Some(text),Some(key)) = (&input.message,&input.message_id) {
                Self::room_key(key)?;
                validate(!text.trim().is_empty() && text.len()<=8000,"Send between 1 and 8,000 bytes of text.")?;
                let previous: Option<(i64,Vec<u8>)> = self.db.query_row("SELECT sequence,text FROM room_messages WHERE participant_id=? AND message_key=?",params![input.participant_id,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some((sequence,cipher)) = previous {
                    let original: String = self.vault.open(&cipher,&format!("room-message:{room}:{sequence}"))?;
                    conflict(&original==text,"This message ID was already used with different text.")?;
                } else {
                    conflict(!channel.closed,"This channel is closed.")?;
                    let sequence: i64 = self.db.query_row("SELECT COALESCE(MAX(sequence),0)+1 FROM room_messages WHERE room_id=?",[room],|r|r.get(0))?;
                    validate(sequence<=20_000,"This channel is full. Open a new channel.")?;
                    self.db.execute("INSERT INTO room_messages(room_id,sequence,participant_id,message_key,text,created_at) VALUES(?,?,?,?,?,?)",params![room,sequence,input.participant_id,key,self.vault.seal(text,&format!("room-message:{room}:{sequence}"))?,self.time()])?;
                }
            }
            self.db.execute("UPDATE room_participants SET last_seen_at=? WHERE id=?",params![self.time(),input.participant_id])?;
            self.room_page(p,room,input.after)
        })
    }
    pub fn room_leave(&self, p: &Principal, room: &str, participant: &str) -> Result<()> {
        self.room(p, room)?;
        self.room_participant(p, room, participant, false)?;
        self.db.execute(
            "UPDATE room_participants SET left_at=? WHERE id=?",
            params![self.time(), participant],
        )?;
        Ok(())
    }
    pub fn room_close(&self, p: &Principal, id: &str) -> Result<Channel> {
        require(
            self.room(p, id)?.owner_id == p.user_id,
            "Only the person who opened this channel can close it.",
        )?;
        self.db
            .execute("UPDATE rooms SET closed=1 WHERE id=?", [id])?;
        self.room(p, id)
    }
}
