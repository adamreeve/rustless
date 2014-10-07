use serialize::json;
use serialize::json::{Json, JsonObject};
use serialize::json::ToJson;

use valico::Builder as ValicoBuilder;
use query;

use server_backend::method::{Method};
use server::{Request, Response};
use middleware::{HandleResult, NotMatchError, Error};
use framework::path::{Path};
use framework::errors::{QueryStringDecodeError, ValidationError, BodyDecodeError};
use framework::{
    ApiHandler, ValicoBuildHandler, Client, CallInfo
};

pub type EndpointHandler = fn<'a>(Client<'a>, &Json) -> HandleResult<Client<'a>>;

pub enum EndpointHandlerPresent {
    HandlerPresent
}

pub type EndpointBuilder = |&mut Endpoint|: 'static -> EndpointHandlerPresent;

#[deriving(Send)]
pub struct Endpoint {
    pub method: Method,
    pub path: Path,
    pub desc: Option<String>,
    pub coercer: Option<ValicoBuilder>,
    handler: Option<EndpointHandler>,
}

impl Endpoint {

    pub fn new(method: Method, path: &str) -> Endpoint {
        Endpoint {
            method: method,
            path: Path::parse(path, true).unwrap(),
            desc: None,
            coercer: None,
            handler: None
        }
    }

    pub fn build(method: Method, path: &str, builder: EndpointBuilder) -> Endpoint {
        let mut endpoint = Endpoint::new(method, path);
        builder(&mut endpoint);

        endpoint
    }

    pub fn desc(&mut self, desc: &str) {
        self.desc = Some(desc.to_string());
    }

    pub fn params(&mut self, builder: ValicoBuildHandler) {
        self.coercer = Some(ValicoBuilder::build(builder));
    }

    pub fn handle(&mut self, handler: EndpointHandler) -> EndpointHandlerPresent {
        self.handler = Some(handler);
        HandlerPresent
    }

    fn validate(&self, params: &mut JsonObject) -> HandleResult<()> {
        // Validate namespace params with valico
        if self.coercer.is_some() {
            // validate and coerce params
            let coercer = self.coercer.as_ref().unwrap();
            match coercer.process(params) {
                Ok(()) => Ok(()),
                Err(err) => return Err(ValidationError{ reason: err }.abstract())
            }   
        } else {
            Ok(())
        }
    }

    pub fn call_decode(&self, params: &mut JsonObject, req: &mut Request, info: &mut CallInfo) -> HandleResult<Response> {
        
        let mut client = Client::new(self, req);

        for cb in info.before.iter() {
            try!((*cb)(&mut client));
        }

        {

        let req: &mut Request = client.request;

        // extend params with query-string params if any
        if req.url.query.is_some() {
            let maybe_query_params = query::parse(req.url.query.as_ref().unwrap().as_slice());
            match maybe_query_params {
                Ok(query_params) => {
                    for (key, value) in query_params.as_object().unwrap().iter() {
                        if !params.contains_key(key) {
                            params.insert(key.to_string(), value.clone());
                        }
                    }
                }, 
                Err(_) => {
                    return Err(QueryStringDecodeError.abstract());
                }
            }
        }

        // extend params with json-encoded body params if any
        if req.is_json_body() {
            let maybe_body = req.read_to_end();
        
            let utf8_string_body = {
                match maybe_body {
                    Ok(body) => {
                        match String::from_utf8(body) {
                            Ok(e) => e,
                            Err(_) => return Err(BodyDecodeError::new("Invalid UTF-8 sequence".to_string()).abstract()),
                        }
                    },
                    Err(err) => return Err(BodyDecodeError::new(format!("{}", err)).abstract())
                }
            };

            if utf8_string_body.len() > 0 {
              let maybe_json_body = json::from_str(utf8_string_body.as_slice());
                match maybe_json_body {
                    Ok(json_body) => {
                        for (key, value) in json_body.as_object().unwrap().iter() {
                            if !params.contains_key(key) {
                                params.insert(key.to_string(), value.clone());
                            }
                        }
                    },
                    Err(err) => return Err(BodyDecodeError::new(format!("{}", err)).abstract())
                }  
            }
        }

        }   

        for cb in info.before_validation.iter() {
            try!((*cb)(&mut client));
        }

        try!(self.validate(params));

        for cb in info.after_validation.iter() {
            try!((*cb)(&mut client));
        }

        let ref handler = self.handler.unwrap();
        // fixme not efficient to_json call
        let mut client = try!((*handler)(client, &params.to_json()));
            
        for cb in info.after.iter() {
            try!((*cb)(&mut client));
        }

        Ok(client.move_response())
    }

}

impl ApiHandler for Endpoint {
    fn api_call(&self, rest_path: &str, params: &mut JsonObject, req: &mut Request, info: &mut CallInfo) -> HandleResult<Response> {

        match self.path.is_match(rest_path) {
            Some(captures) =>  {
                self.path.apply_captures(params, captures);
                self.call_decode(params, req, info)
            },
            None => Err(NotMatchError.abstract())
        }

    }
}