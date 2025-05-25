




pub struct ParticipantBuffer {
    data: Vec<u8>,
    participants_meta: ParticipantInfo,
}


impl ParticipantBuffer {


    pub fn new();
    pub fn add_raw(&mut self, participants: Vec<u8>) {
        todo!();
    }

    pub fn add<T: Participants>(&mut self, participants: T) {
        todo!();
    }
}


//TODO: Iter through buffer